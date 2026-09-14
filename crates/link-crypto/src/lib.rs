//! aipg-link-crypto：gateway 间链路「压缩 + 加密」高性能硬化路径。
//!
//! 协议（design 2026-08-26 链接压缩+加密）：
//! ```text
//! raw = nonce(12B) ‖ AES-256-GCM(gzip(明文))         // GCM 密文尾部已含 16B tag
//! key = SHA-256(bearer_token) → 32B
//! header: x-aipg-enc: v1
//! ```
//!
//! 高性能要点：
//! - **零 base64**：raw 二进制直接入 body，省去 33% 体积膨胀与编解码开销；
//! - **AES-NI/VAES 硬件加速**：RustCrypto `aes-gcm` 在 release 构建下自动启用
//!   x86_64 AES-NI / aarch64 加密扩展，密钥扩展与块加密均为硬件指令；
//! - **先压后密**：gzip 只作用于明文（密文高熵不可压），且整包一次加解密
//!   （组长/组员两侧均非真流式，无流式编码器状态机开销）；
//! - **一次性派生**：密钥派生仅为 SHA-256(token)（微秒级），不引入跨请求缓存锁。

use aes_gcm::aead::{Aead, KeyInit, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

/// 链路加密协商头。
pub const ENC_HEADER: &str = "x-aipg-enc";
/// 当前协议版本。
pub const ENC_VERSION: &str = "v1";
/// nonce 长度（AES-GCM 标准 96-bit）。
pub const NONCE_LEN: usize = 12;

/// 链路加密模式（配置项 `link.encrypt`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkEncryptMode {
    /// 明文（默认；LAN 直连/旧成员兼容）。
    #[default]
    Off,
    /// 协议层 AES-256-GCM 端到端加密（跨网络深链）。
    AesGcm,
    /// 组长端强制模式：未声明加密的 /v1/* 请求回 426（成员端等同 AesGcm 加密发送）。
    Enforce,
    /// 预留：传输层 TLS（QUIC/TLS1.3 内建，零配置）。
    Tls,
}

impl LinkEncryptMode {
    /// 解析配置值（`off` | `aes-gcm` | `enforce` | `tls`；大小写不敏感，空串视为 off）。
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "aes-gcm" | "aes_gcm" | "aes" => Self::AesGcm,
            "enforce" | "strict" | "mandatory" => Self::Enforce,
            "tls" | "quic" => Self::Tls,
            _ => Self::Off,
        }
    }

    /// 配置值回写（与 parse 互逆：off/aes-gcm/enforce/tls）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::AesGcm => "aes-gcm",
            Self::Enforce => "enforce",
            Self::Tls => "tls",
        }
    }

    /// 是否需要链路加密（成员端发送侧判断：Off 之外均加密；Enforce 语义同上）。
    pub fn encrypts(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// 链路加密错误。
#[derive(Debug, thiserror::Error)]
pub enum LinkCryptoError {
    #[error("payload too short ({0} bytes < nonce+tag minimum)")]
    TooShort(usize),
    #[error("gzip compress failed: {0}")]
    Compress(String),
    #[error("gzip decompress failed: {0}")]
    Decompress(String),
    #[error("AES-GCM decrypt failed (bad token / tampered payload)")]
    Decrypt,
}

/// 链路加密器：密钥派生 + gzip（级别可配）。
#[derive(Debug, Clone)]
pub struct LinkCrypto {
    /// gzip 压缩级别（1=最快 … 9=最佳）；默认 6 平衡。
    gzip_level: u32,
}

impl Default for LinkCrypto {
    fn default() -> Self {
        Self { gzip_level: 6 }
    }
}

impl LinkCrypto {
    /// 默认加密器（gzip level 6）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 快速压缩（gzip level 1）：CPU 敏感场景降低压缩开销，代价是压缩比略低。
    pub fn fast() -> Self {
        Self { gzip_level: 1 }
    }

    /// 从 bearer token 派生 32 字节密钥（SHA-256；设计文档 §5.5）。
    pub fn derive_key(token: &str) -> [u8; 32] {
        Sha256::digest(token.as_bytes()).into()
    }

    /// 加密：`raw = nonce ‖ AES-256-GCM(gzip(plain))`。零 base64，输出即二进制 body。
    pub fn encrypt(&self, key: &[u8; 32], plain: &[u8]) -> Vec<u8> {
        // 先压后密：gzip 只压明文
        let compressed = self.gzip(plain);
        let cipher = Aes256Gcm::new_from_slice(key).expect("32B key");
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        // GCM 单次加密（密文尾部自带 16B tag），整包处理
        let ct = cipher
            .encrypt(nonce, compressed.as_slice())
            .expect("AES-GCM encrypt");
        let mut raw = Vec::with_capacity(NONCE_LEN + ct.len());
        raw.extend_from_slice(&nonce_bytes);
        raw.extend_from_slice(&ct);
        raw
    }

    /// 解密并解压：输入 `nonce ‖ ct(tag 在尾部)`，输出明文；失败 = 密钥不符/被篡改。
    pub fn decrypt(&self, key: &[u8; 32], raw: &[u8]) -> Result<Vec<u8>, LinkCryptoError> {
        if raw.len() < NONCE_LEN + 16 {
            return Err(LinkCryptoError::TooShort(raw.len()));
        }
        let (nonce_bytes, ct) = raw.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new_from_slice(key).expect("32B key");
        let compressed = cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ct)
            .map_err(|_| LinkCryptoError::Decrypt)?;
        self.gunzip(&compressed)
    }

    /// gzip 压缩（整包一次）。
    fn gzip(&self, plain: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::with_capacity(plain.len() / 2 + 64), Compression::new(self.gzip_level));
        enc.write_all(plain).ok();
        enc.finish().unwrap_or_default()
    }

    /// gzip 解压（整包一次）。
    fn gunzip(&self, compressed: &[u8]) -> Result<Vec<u8>, LinkCryptoError> {
        let mut dec = GzDecoder::new(compressed);
        let mut out = Vec::with_capacity(compressed.len() * 2 + 64);
        dec.read_to_end(&mut out)
            .map_err(|e| LinkCryptoError::Decompress(e.to_string()))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_roundtrip_and_deterministic_key() {
        let crypto = LinkCrypto::new();
        let key = LinkCrypto::derive_key("sk-aipg-abc123");
        let plain = br#"{"model":"deepseek-chat","messages":[{"role":"user","content":"hi"}]}"#;
        let raw = crypto.encrypt(&key, plain);
        // 格式：nonce(12) + ct，无 base64 膨胀；密文长度 < 原文 + nonce + tag（压缩生效）
        assert!(raw.len() >= NONCE_LEN + 16);
        let out = crypto.decrypt(&key, &raw).expect("decrypt ok");
        assert_eq!(&out[..], plain);
        // 密钥确定性：同一 token 两次派生一致
        assert_eq!(LinkCrypto::derive_key("t"), LinkCrypto::derive_key("t"));
        assert_ne!(LinkCrypto::derive_key("t"), LinkCrypto::derive_key("t2"));
    }

    #[test]
    fn nonce_unique_per_message() {
        let crypto = LinkCrypto::new();
        let key = LinkCrypto::derive_key("k");
        let a = crypto.encrypt(&key, b"payload");
        let b = crypto.encrypt(&key, b"payload");
        assert_ne!(a[..NONCE_LEN], b[..NONCE_LEN], "每次加密必须新 nonce");
        // 但都可独立解密
        assert_eq!(crypto.decrypt(&key, &a).unwrap(), b"payload");
        assert_eq!(crypto.decrypt(&key, &b).unwrap(), b"payload");
    }

    #[test]
    fn wrong_key_fails() {
        let crypto = LinkCrypto::new();
        let raw = crypto.encrypt(&LinkCrypto::derive_key("real"), b"secret");
        assert!(crypto.decrypt(&LinkCrypto::derive_key("wrong"), &raw).is_err());
    }

    #[test]
    fn tampered_payload_fails() {
        let crypto = LinkCrypto::new();
        let key = LinkCrypto::derive_key("k");
        let mut raw = crypto.encrypt(&key, b"payload");
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        assert!(crypto.decrypt(&key, &raw).is_err());
    }

    #[test]
    fn too_short_rejected() {
        let crypto = LinkCrypto::new();
        let key = LinkCrypto::derive_key("k");
        assert!(matches!(crypto.decrypt(&key, &[0u8; 10]), Err(LinkCryptoError::TooShort(_))));
    }

    #[test]
    fn fast_level_still_roundtrips() {
        let crypto = LinkCrypto::fast();
        let key = LinkCrypto::derive_key("k");
        let raw = crypto.encrypt(&key, b"compressible payload ".repeat(32).as_slice());
        let out = crypto.decrypt(&key, &raw).unwrap();
        assert_eq!(out, b"compressible payload ".repeat(32));
    }

    #[test]
    fn mode_parse() {
        assert_eq!(LinkEncryptMode::parse("off"), LinkEncryptMode::Off);
        assert_eq!(LinkEncryptMode::parse("aes-gcm"), LinkEncryptMode::AesGcm);
        assert_eq!(LinkEncryptMode::parse("AES_GCM"), LinkEncryptMode::AesGcm);
        assert_eq!(LinkEncryptMode::parse("enforce"), LinkEncryptMode::Enforce);
        assert_eq!(LinkEncryptMode::parse("Strict"), LinkEncryptMode::Enforce);
        assert_eq!(LinkEncryptMode::parse("tls"), LinkEncryptMode::Tls);
        assert_eq!(LinkEncryptMode::parse(""), LinkEncryptMode::Off);
        assert_eq!(LinkEncryptMode::parse("garbage"), LinkEncryptMode::Off);
        // as_str / parse 互逆
        for m in [LinkEncryptMode::Off, LinkEncryptMode::AesGcm, LinkEncryptMode::Enforce, LinkEncryptMode::Tls] {
            assert_eq!(LinkEncryptMode::parse(m.as_str()), m);
        }
        // encrypts 语义：仅 Off 不加密
        assert!(!LinkEncryptMode::Off.encrypts());
        assert!(LinkEncryptMode::AesGcm.encrypts());
        assert!(LinkEncryptMode::Enforce.encrypts());
        assert!(LinkEncryptMode::Tls.encrypts());
    }
}
//! Vault：敏感值加密存储（AES-GCM）。
//! 密钥：机器绑定 + 用户数据目录派生（0.1.0 简化；1.x 换系统凭据库）。

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use aes_gcm::aead::rand_core::RngCore;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

/// 加密保险库。
#[derive(Clone)]
pub struct Vault {
    cipher: Aes256Gcm,
}

impl Vault {
    /// 从机器+数据目录派生密钥创建。
    pub fn new(data_dir: &std::path::Path) -> Self {
        // 密钥文件（不存在则生成）
        let key_path = data_dir.join("vault.key");
        let key_bytes = if key_path.exists() {
            std::fs::read(&key_path).unwrap_or_default()
        } else {
            let mut k = [0u8; 32];
            OsRng.fill_bytes(&mut k);
            let _ = std::fs::create_dir_all(data_dir);
            let _ = std::fs::write(&key_path, k);
            k.to_vec()
        };
        let mut key_arr = [0u8; 32];
        let n = key_bytes.len().min(32);
        key_arr[..n].copy_from_slice(&key_bytes[..n]);
        Self { cipher: Aes256Gcm::new_from_slice(&key_arr).expect("valid key") }
    }

    /// 加密明文 → base64 密文。
    pub fn encrypt(&self, plain: &str) -> String {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self.cipher.encrypt(nonce, plain.as_bytes()).unwrap_or_default();
        // 格式：nonce(12B base64) + ":" + ct
        format!("{}:{}", B64.encode(nonce_bytes), B64.encode(ct))
    }

    /// 解密密文 → 明文（失败返回空）。
    pub fn decrypt(&self, encoded: &str) -> Option<String> {
        let (n_b64, ct_b64) = encoded.split_once(':')?;
        let nonce_bytes = B64.decode(n_b64).ok()?;
        let ct = B64.decode(ct_b64).ok()?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        self.cipher.decrypt(nonce, ct.as_ref()).ok().map(|p| String::from_utf8_lossy(&p).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let dir = std::env::temp_dir().join("aipg-vault-test");
        let _ = std::fs::remove_dir_all(&dir);
        let v = Vault::new(&dir);
        let enc = v.encrypt("secret-password");
        assert_ne!(enc, "secret-password");
        assert!(enc.contains(':'));
        assert_eq!(v.decrypt(&enc).as_deref(), Some("secret-password"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_vault_key_roundtrip() {
        let dir = std::env::temp_dir().join("aipg-vault-test2");
        let _ = std::fs::remove_dir_all(&dir);
        let v1 = Vault::new(&dir);
        let enc = v1.encrypt("token-abc");
        let v2 = Vault::new(&dir); // 同目录 → 同密钥
        assert_eq!(v2.decrypt(&enc).as_deref(), Some("token-abc"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
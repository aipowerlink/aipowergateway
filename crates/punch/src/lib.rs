//! aipg-punch: UDP 打洞 + QUIC 承载（跨域 P2P 直连，路径2）。
//!
//! 分层：
//! - [`candidate`]：打洞候选（本地绑定地址 + STUN 反射的公网映射地址）
//! - [`punch`]：双向 UDP 打洞（互发探测/回显，确认 NAT 双向可达）
//! - [`quic`]：打洞成功后在同一 socket 上承载 QUIC（组长自签证书，
//!   指纹经信令交成员校验；数据面 = 成员直连组长，不经服务器）
//!
//! 约束（零知识契约 §5）：服务器只做信令（候选交换），不承载流量；
//! 打洞失败 = 直连失败，客户端明确报错，无中继兜底。
//!
//! 打洞流程：
//! 1. 双方各绑 UDP socket，收集候选（local + mapped[STUN 反射]）；
//! 2. 经协调服务器 `/v1/signal` 交换候选（成员→组长→成员）；
//! 3. 双方同时向对方候选互发探测包（AIPG tag 帧），收到对方包即双向可达；
//! 4. 组长把同一 socket 交给 quinn server 监听，成员 connect（打洞建立的
//!    公网映射地址），QUIC 握手/数据面全部走已打通的 UDP 通路。

pub mod candidate;
pub mod error;
pub mod punch;
pub mod quic;
pub mod tunnel;

pub use candidate::{Candidate, CandidateKind};
pub use error::{Error, Result};
pub use punch::{PunchOptions, PunchSocket};
pub use quic::{PunchCert, connect, leader_endpoint, member_endpoint};
pub use tunnel::{QuicBiStream, bridge_bi_to_tcp, run_leader_tunnel, spawn_member_tunnel};
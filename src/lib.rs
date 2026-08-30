mod api;
mod client;
mod config;
mod dns;
mod qrcode;
mod resp;
mod session;
mod state;
mod template;
mod totp;
mod utils;
mod wg;

pub use client::{get_company_url, Client};
pub use config::{
    Config, RouteMode, WgConf, PLATFORM_CORPLINK_V1, PLATFORM_LARK, PLATFORM_OIDC,
};
pub use dns::DNSManager;
pub use resp::{RespCompany, RespVpnInfo};
pub use session::{
    ConnectionAttempt, ConnectionInfo, InvalidTransition, QrChallenge, QrPollStatus, SessionEvent,
    SessionFailure, SessionState, TunnelMode,
};
pub use wg::{start_wg_go, start_wg_go_netstack, stop_wg_go, UAPIClient, WgStats};

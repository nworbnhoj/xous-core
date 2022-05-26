#![cfg_attr(target_os = "none", no_std)]

pub mod api;
pub use api::{Opcode, WebsocketConfig, SERVER_NAME_WEBSOCKET, SUB_PROTOCOL_LEN};

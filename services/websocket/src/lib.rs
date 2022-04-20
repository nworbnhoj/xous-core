#![cfg_attr(target_os = "none", no_std)]

pub mod api;
pub use api::{Opcode, Return, WebsocketConfig, SERVER_NAME_WEBSOCKET, CA_LEN, SUB_PROTOCOL_LEN};


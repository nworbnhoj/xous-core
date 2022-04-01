#[allow(dead_code)]
pub(crate) const SERVER_NAME_WEBSOCKET: &str = "_Websocket Service_";

use embedded_websocket::WebSocketSendMessageType;
use std::time::Duration;

pub(crate) const KEEPALIVE_TIMEOUT_SECONDS: Duration = Duration::from_secs(55);
pub(crate) const URL_LENGTH_LIMIT: usize = 200;

pub(crate) const BASEURL_LEN: usize = 128;
pub(crate) const PATH_LEN: usize = 128;
pub(crate) const LOGIN_LEN: usize = 128;
pub(crate) const PASSWORD_LEN: usize = 128;

/*
 WEBSOCKET_BUFFER_LEN can be as small as 14bytes, but presumably comes with a performance degradation.
 ( see https://crates.io/crates/embedded-websocket )
 Also: there may be advantage in independently specifying the read, frame, and write buffer sizes.
 TODO review/test/optimise WEBSOCKET_BUFFER_LEN
*/
pub(crate) const WEBSOCKET_BUFFER_LEN: usize = 4000;

/// These opcodes can be called by anyone at any time
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub(crate) enum Opcode {
    /// close and existing websocket
    Close = 1,
    /// open a new websocket
    Open,
    /// send a websocket frame
    Send,
    /// send a KeepAliveRequest
    Tick,
    /// Close Websocket and shutdown server
    Quit,
}

// Subset of use embedded_websocket::WebSocketSendMessageType
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum SendMessageType {
    Text,
    Binary,
}

// a structure for defining the setup of a Websocket.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct WebsocketConfig {
    //    pub tls_config: rustls::ClientConfig,   // requires manual implementation of Archive....
    pub base_url: xous_ipc::String<BASEURL_LEN>,
    pub path: xous_ipc::String<PATH_LEN>,
    pub use_credentials: bool,
    pub login: xous_ipc::String<LOGIN_LEN>,
    pub password: xous_ipc::String<PASSWORD_LEN>,
}

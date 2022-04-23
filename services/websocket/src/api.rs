/// The websocket service can open, maintain and close a websocket connection.
/// The service can also send data and regularly polls the connection for inbound
/// data to forward to the CID provided when the websocket was opened.

#[allow(dead_code)]
pub const SERVER_NAME_WEBSOCKET: &str = "_Websocket Service_";

use std::time::Duration;

/** time between reglar websocket keep-alive requests */
pub(crate) const KEEPALIVE_TIMEOUT_SECONDS: Duration = Duration::from_secs(55);
/** time between regulr pol for inboud frames on open websockets */
pub(crate) const LISTENER_POLL_INTERVAL_MS: Duration = Duration::from_millis(250);
/** limit on the byte length of url strings */
pub(crate) const URL_LENGTH_LIMIT: usize = 200;
/** limit on the byte length of error hint strings */
pub(crate) const HINT_LEN: usize = 128;
/** limit on the byte length of certificate authority strings */
pub const CA_LEN: usize = 1402;
/** limit on the byte length of base-url strings */
pub(crate) const BASEURL_LEN: usize = 128;
/** limit on the byte length of url path strings */
pub(crate) const PATH_LEN: usize = 128;
/** limit on the byte length of authentication login strings */
pub(crate) const LOGIN_LEN: usize = 128;
/** limit on the byte length of authentication password strings */
pub(crate) const PASSWORD_LEN: usize = 128;
/** limit on the byte length of websocket sub-protocol strings */
pub const SUB_PROTOCOL_LEN: usize = 24;

/*
 WEBSOCKET_BUFFER_LEN can be as small as 14bytes, but presumably comes with a performance degradation.
 ( see https://crates.io/crates/embedded-websocket )
 Also: there may be advantage in independently specifying the read, frame, and write buffer sizes.
 TODO review/test/optimise WEBSOCKET_BUFFER_LEN
*/
pub(crate) const WEBSOCKET_BUFFER_LEN: usize = 4096;

/// These opcodes can be called by anyone at any time
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum Opcode {
    /// Close an existing websocket.
    /// xous::Message::new_scalar(Opcode::Close, _, _, _, _)
    Close = 1,
    /// Open a new websocket.
    /// Attempts to establish a new websocket connection based on WebsocketConfig and return
    /// the sub_protocol nominated by the server (if any).
    ///     let ws_config = WebsocketConfig {
    ///         certificate_authority:     optional ca for a TLS connection - fallback to tcp
    ///         base_url:                  the url of the target websocket server
    ///         path:                      a path to apend to the url
    ///         use_credentials:           true to authenticate
    ///         login:                     authentication username
    ///         password:                  authentication password
    ///         cid:                       the callback id for inbound data frames
    ///         opcode:                    the opcode for inbound data frames
    ///     };
    ///     let buf = Buffer::into_buf(ws_config);
    ///     buf.lend(ws_cid, Opcode::Open).map(|_| ());
    ///     let sub_protocol: Return::SubProtocol(protocol) = buf.to_original::<Return, _>().unwrap()
    Open,
    /// Poll websockets for inbound frames.
    /// An independent background thread is spawned to pump a regular Poll (LISTENER_POLL_INTERVAL_MS)
    /// so there is normally no need to call this Opcode.
    /// xous::Message::new_scalar(Opcode::Poll, _, _, _, _)
    Poll,
    /// send a websocket frame
    Send,
    /// Return the current State of the websocket
    /// 1=Open, 0=notOpen
    /// xous::Message::new_scalar(Opcode::State, _, _, _, _)
    State,
    /// Send a KeepAliveRequest.
    /// An independent background thread is spawned to pump a regular Tick (KEEPALIVE_TIMEOUT_SECONDS)
    /// so there is normally no need to call this Opcode.
    /// xous::Message::new_scalar(Opcode::Tick, _, _, _, _)
    Tick,
    /// Close all websockets and shutdown server
    /// xous::Message::new_scalar(Opcode::Quit, _, _, _, _)
    Quit,
}

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug, PartialEq)]
pub(crate) enum WsError {
    /// This Opcode accepts Scalar calls
    Scalar,
    /// This Opcode accepts Blocking Scalar calls
    ScalarBlock,
    /// This Opcode accepts Memory calls
    Memory,
    /// This Opcode accepts Blocking Memory calls
    MemoryBlock,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum Return {
    SubProtocol(xous_ipc::String<SUB_PROTOCOL_LEN>),
    Failure(xous_ipc::String<HINT_LEN>),
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Frame {
    pub bytes: [u8; WEBSOCKET_BUFFER_LEN],
}

// Subset of use embedded_websocket::WebSocketSendMessageType
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum SendMessageType {
    Text,
    Binary,
}

// a structure for defining the setup of a Websocket.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Debug)]
pub struct WebsocketConfig {
    /** optional ca for a TLS connection - fallback to tcp */
    pub certificate_authority: Option<xous_ipc::String<CA_LEN>>,
    /** the url of the target websocket server */
    pub base_url: xous_ipc::String<BASEURL_LEN>,
    /** a path to apend to the url */
    pub path: xous_ipc::String<PATH_LEN>,
    /** true to authenticate */
    pub sub_protocols: [xous_ipc::String<SUB_PROTOCOL_LEN>; 3],
    pub use_credentials: bool,
    /** authentication username */
    pub login: xous_ipc::String<LOGIN_LEN>,
    /** authentication password */
    pub password: xous_ipc::String<PASSWORD_LEN>,
    /** the callback id for inbound data frames */
    pub cid: u32,
    /** the opcode for inbound data frames */
    pub opcode: u32,
}

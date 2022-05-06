/// The websocket service can open, maintain and close a websocket connection.
/// The service can also send data and regularly polls the connection for inbound
/// data to forward to the CID provided when the websocket was opened.
use std::io::Error;
use std::net::TcpStream;
use embedded_websocket as ws;
use rustls::{ClientConnection, StreamOwned};

#[allow(dead_code)]
pub const SERVER_NAME_WEBSOCKET: &str = "_Websocket Service_";

/** limit on the byte length of url strings */
pub(crate) const URL_LENGTH_LIMIT: usize = 200;
/** limit on the byte length of error hint strings */
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
pub(crate) const HINT_LEN: usize = 128;

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
    /// send a websocket frame
    Send,
    /// Return the current State of the websocket
    /// 1=Open, 0=notOpen
    /// xous::Message::new_scalar(Opcode::State, _, _, _, _)
    State,
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
    /// Websocket assets corruption
    AssetsFault,
    /// Error in Websocket protocol
    ProtocolError,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum Return {
    SubProtocol(Option<xous_ipc::String<SUB_PROTOCOL_LEN>>),
    Failure(xous_ipc::String<HINT_LEN>),
}

// a structure for defining the setup of a Websocket.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Debug, Clone, Copy)]
pub struct WebsocketConfig {
    /** the callback id for inbound data frames */
    pub cid: u32,
    /** the opcode for inbound data frames */
    pub opcode: u32,
    /** the url of the target websocket server */
    pub host: xous_ipc::String<BASEURL_LEN>,
    /** the port on the target websocket server */
    pub port: Option<xous_ipc::String<BASEURL_LEN>>,
    /** a path to apend to the url */
    pub path: Option<xous_ipc::String<PATH_LEN>>,
    /** authentication username */
    pub login: Option<xous_ipc::String<LOGIN_LEN>>,
    /** authentication password */
    pub password: Option<xous_ipc::String<PASSWORD_LEN>>,
    /** optional ca for a TLS connection - fallback to tcp */
    pub certificate_authority: Option<xous_ipc::String<CA_LEN>>,
    /** websocket sub-protocols max 3*/
    pub	sub_protocol: Option<xous_ipc::String<SUB_PROTOCOL_LEN>>,
}

#[derive(Debug)]
pub(crate) enum WsStream {
    Tcp(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}

impl ws::framer::Stream<Error> for WsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, Error> {
        match self {
            WsStream::Tcp(s) => std::io::Read::read(s, buf),
            WsStream::Tls(s) => std::io::Read::read(s, buf),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> std::result::Result<(), Error> {
        match self {
            WsStream::Tcp(s) => std::io::Write::write_all(s, buf),
            WsStream::Tls(s) => std::io::Write::write_all(s, buf),
        }
    }
}



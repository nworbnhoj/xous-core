use rustls::{ClientConnection, StreamOwned};
/// The websocket server can open, maintain and close a websocket connection.
/// The server can also send data and regularly polls the connection for inbound
/// data to forward to the CID provided when the websocket was opened.
use std::io::Error;
use std::net::TcpStream;

use embedded_websocket as ws;

//use rand_chacha::rand_core::OsRng;

pub const SUB_PROTOCOL_LEN: usize = 24;
pub(crate) const HINT_LEN: usize = 128;

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum Opcode {
    /// Close an existing websocket.
    /// xous::Message::new_scalar(Opcode::Close, _, _, _, _)
    Close = 1,
    /// poll open Websockets for inbound frames
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

impl Into<usize> for Opcode {
    fn into(self) -> usize {
        self as usize
    }
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

#[repr(C, align(4096))]
pub struct LendData(pub [u8; 4096]);

#[repr(C, align(4096))]
pub struct SendData(pub [u8; 4096]);

impl Drop for SendData {
    fn drop(&mut self) {} // dont drop!
}

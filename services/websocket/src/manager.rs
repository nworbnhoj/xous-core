#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

use super::*;
use crate::api::{WsError, WsStream, SUB_PROTOCOL_LEN};

use embedded_websocket as ws;
use num_traits::{FromPrimitive, ToPrimitive};
use rand::rngs::OsRng;

use rustls_connector::*;
use std::io::{Error, ErrorKind};
use std::num::NonZeroU8;
use std::{collections::HashMap, convert::TryInto, net::TcpStream, thread};
use ws::framer::Framer;
use ws::WebSocketCloseStatusCode as StatusCode;
use ws::WebSocketSendMessageType as MessageType;
use ws::{WebSocketClient, WebSocketOptions, WebSocketState};
use xous::{CID, SID};
use xous_ipc::Buffer;

use url::Url;

use std::time::Duration;

/** time between reglar websocket keep-alive requests */
pub(crate) const KEEPALIVE_TIMEOUT_SECONDS: Duration = Duration::from_secs(55);
/** time between regular poll for inbound frames */
pub(crate) const LISTENER_POLL_INTERVAL_MS: Duration = Duration::from_millis(250);
pub(crate) const HINT_LEN: usize = 128;
/** limit on the byte length of certificate authority strings */
/*
 A websocket header requires at least 14 bytes of the websocket buffer
 ( see https://crates.io/crates/embedded-websocket ) leaving the remainder
 available for the payload. This relates directly to the frame buffer.
 There may be advantage in independently specifying the read, frame, and write buffer sizes.
 TODO review/test/optimise WEBSOCKET_BUFFER_LEN
*/
pub(crate) const WEBSOCKET_BUFFER_LEN: usize = 4096;
pub(crate) const WEBSOCKET_PAYLOAD_LEN: usize = 4080;

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Frame {
    pub bytes: [u8; WEBSOCKET_PAYLOAD_LEN],
}

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum Opcode {
    /// Close an existing websocket.
    /// xous::Message::new_scalar(Opcode::Close, _, _, _, _)
    Close = 1,
    ///
    Open,
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

pub(crate) struct Client {
    /** the configuration of an open websocket */
    socket: WebSocketClient<OsRng>,
    /** a websocket stream when opened on a tcp connection */
    ws_stream: WsStream,
    /** the underlying tcp stream */
    tcp_stream: TcpStream,
    /** the framer read buffer */
    read_buf: [u8; WEBSOCKET_BUFFER_LEN],
    /** the framer read cursor */
    read_cursor: usize,
    /** the framer write buffer */
    write_buf: [u8; WEBSOCKET_BUFFER_LEN],
    /** the callback_id to use when relaying an inbound websocket frame */
    cid: CID,
    /** the opcode to use when relaying an inbound websocket frame */
    opcode: u32,
    /** **/
    sub_protocol: Option<xous_ipc::String<SUB_PROTOCOL_LEN>>,
}

impl Client {
    pub(crate) fn new(ws_config: WebsocketConfig, cid: CID, opcode: u32) -> Result<Self, Error> {
        log::info!("Configuring new WebSocket");
        // construct url from ws_config
        let host = ws_config.host.as_str().expect("url utf-8 decode fail");
        let mut url = Url::parse(host).expect("invalid host");
        let path = match ws_config.path {
            Some(path) => {
                let path = path.as_str().expect("path utf-8 decode error");
                url = url.join(path).expect("valid path");
                path.to_string()
            }
            None => "".to_string(),
        };
        if ws_config.port.is_some() {
            match ws_config.port.unwrap().to_string().parse() {
                Ok(int) => url.set_port(Some(int)).unwrap(),
                Err(e) => {
                    log::warn!("failed to parse websocket port");
                }
            }
        }
        match ws_config.login {
            Some(login) => {
                let login = login.as_str().expect("login utf-8 decode error");
                url.query_pairs_mut().append_pair("login", &login);
            }
            None => {}
        }
        match ws_config.password {
            Some(pwd) => {
                let password = pwd.as_str().expect("password utf-8 decode error");
                url.query_pairs_mut().append_pair("password", password);
            }
            None => {}
        }
        match ws_config.certificate_authority.is_some() {
            true => url.set_scheme("wss").expect("fail set url scheme"),
            false => url.set_scheme("ws").expect("fail set url scheme"),
        };

        // Create a TCP Stream between this device and the remote Server
        log::info!("Opening TCP connection to {:?}", host);
        let mut tcp_stream = match TcpStream::connect(url.as_str()) {
            Ok(tcp_stream) => tcp_stream,
            Err(e) => {
                log::warn!("Failed to open TCP Stream {:?}", e);
                return Err(Error::from(ErrorKind::ConnectionRefused));
            }
        };
        let tcp_clone = tcp_stream.try_clone().expect("Failed to clone TCP Stream");
        log::info!("TCP connected to {:?}", host);

        let mut ws_stream = match ws_config.certificate_authority {
            None => WsStream::Tcp(tcp_stream),
            Some(ca) => {
                // Create a TLS connection to the remote Server on the TCP Stream
                let ca = ca
                    .as_str()
                    .expect("certificate_authority utf-8 decode error");

                // setup the ssl certificate
                let mut cert_bytes = std::io::Cursor::new(ca);
                let roots = rustls_pemfile::certs(&mut cert_bytes).expect("parseable PEM files");
                let roots = roots.iter().map(|v| rustls::Certificate(v.clone()));

                let mut root_certs = rustls::RootCertStore::empty();
                for root in roots {
                    root_certs.add(&root).unwrap();
                }

                let ssl_config = rustls::ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(root_certs)
                    .with_no_client_auth();

                let tls_connector = RustlsConnector::from(ssl_config);
                let tls_stream = tls_connector
                    .connect(url.host_str().unwrap(), tcp_stream)
                    .expect("Failed to complete TLS handshake");
                log::info!("TLS connected to {:?}", url.host_str().unwrap());
                WsStream::Tls(tls_stream)
            }
        };

        let sub_protocol;
        let mut sub_protocol_arr: [&str; 1] = [""];
        let sub_protocols: Option<&[&str]> = match ws_config.sub_protocol {
            Some(sp) => {
                sub_protocol = sp
                    .as_str()
                    .expect("sub_protocol 0 utf-8 decode error")
                    .to_string();
                sub_protocol_arr[0] = &sub_protocol;
                Some(&sub_protocol_arr[..])
            }
            None => None,
        };

        // Prepare for a websocket connection
        let mut read_buf = [0u8; WEBSOCKET_BUFFER_LEN];
        let mut read_cursor = 0;
        let mut write_buf = [0u8; WEBSOCKET_BUFFER_LEN];
        let mut ws_client = WebSocketClient::new_client(OsRng);
        let mut framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );

        let websocket_options = WebSocketOptions {
            path: &path,
            host: host,
            origin: &url.origin().unicode_serialization(),
            sub_protocols: sub_protocols,
            additional_headers: None,
        };

        // Initiate a websocket opening handshake over the TLS Stream
        let sub_protocol = match framer.connect(&mut ws_stream, &websocket_options) {
            Ok(opt) => match opt {
                Some(sp) => Some(xous_ipc::String::from_str(sp.to_string())),
                None => None,
            },
            Err(e) => {
                log::warn!("Unable to connect WebSocket {:?}", e);
                return Err(Error::from(ErrorKind::ConnectionRefused));
            }
        };

        log::info!("WebSocket connected with protocol: {:?}", sub_protocol);

        Ok(Client {
            socket: ws_client,
            ws_stream,
            tcp_stream: tcp_clone,
            read_buf,
            read_cursor,
            write_buf,
            cid,
            opcode,
            sub_protocol,
        })
    }

    pub(crate) fn Ok(&self) -> bool {
        //TODO fix framer status
        //self.framer.status() == WebSocketState::Open
        true
    }

    fn empty(&self) -> bool {
        self.tcp_stream
            .set_nonblocking(true)
            .expect("failed to set TCP Stream to non-blocking");
        let mut frame_buf = [0u8; 8];
        let empty = match self.tcp_stream.peek(&mut frame_buf) {
            Ok(_) => false,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => true,
            Err(e) => {
                log::warn!("TCP IO error: {}", e);
                true
            }
        };
        self.tcp_stream
            .set_nonblocking(false)
            .expect("failed to set TCP Stream to non-blocking");
        empty
    }

    /** read all available frames from the websocket and relay each frame to the caller_id */
    fn read(&mut self) {

        let mut framer = Framer::new(
            &mut self.read_buf[..],
            &mut self.read_cursor,
            &mut self.write_buf[..],
            &mut self.socket,
        );
        let mut frame_buf = [0u8; WEBSOCKET_PAYLOAD_LEN];

        while let Some(frame) = framer
            .read_binary(&mut self.ws_stream, &mut frame_buf[..])
            .expect("failed to read websocket")
        {
            let frame: [u8; WEBSOCKET_PAYLOAD_LEN] = frame
                .try_into()
                .expect("websocket frame too large for buffer");
            let buf = Buffer::into_buf(Frame { bytes: frame })
                .expect("failed to serialize websocket frame into buffer");
            buf.send(self.cid, self.opcode)
                .expect("failed to relay websocket frame");
        }
    }

    fn write(&mut self, buffer: &[u8]) -> Result<(), Error> {
        let mut ret = Ok(());

        let mut framer = Framer::new(
            &mut self.read_buf[..],
            &mut self.read_cursor,
            &mut self.write_buf[..],
            &mut self.socket,
        );

        let mut end_of_message = false;
        let mut start = 0;
        let mut slice;
        while !end_of_message {
            log::info!("start = {:?}", start);
            if buffer.len() < (start + WEBSOCKET_PAYLOAD_LEN) {
                end_of_message = true;
                slice = &buffer[start..];
            } else {
                slice = &buffer[start..(start + WEBSOCKET_PAYLOAD_LEN)];
            }
            let ret = match framer.write(
                &mut self.ws_stream,
                MessageType::Binary,
                end_of_message,
                slice,
            ) {
                Ok(ret) => (),
                Err(e) => {
                    return Err(Error::new(
                        ErrorKind::BrokenPipe,
                        "Failed to write websocket frame",
                    ))
                }
            };
            start = start + WEBSOCKET_PAYLOAD_LEN;
        }
        ret
    }

    fn close(&mut self) -> Result<(), Error> {
        let mut framer = Framer::new(
            &mut self.read_buf[..],
            &mut self.read_cursor,
            &mut self.write_buf[..],
            &mut self.socket,
        );

        match framer.close(&mut self.ws_stream, StatusCode::NormalClosure, None) {
            Ok(ret) => Ok(()),
            Err(e) => {
                log::warn!("Failed to close WebSocket {:?}", e);
                return Err(Error::from(ErrorKind::Other));
            }
        }
    }
}

pub(crate) fn main(sid: SID) -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let cid = xous::connect(sid).unwrap();

    // build a thread that emits a regular Opcode::Tick to send a KeepAliveRequest
    spawn_tick_pump(cid);

    // build a thread that emits a regular Opcode::Poll to receive inbound frames
    spawn_poll_pump(cid);

    /* holds the assets of existing websockets by pid - and as such - limits each pid to 1 websocket. */
    // TODO review the limitation of 1 websocket per pid.
    let mut clients: HashMap<NonZeroU8, Client> = HashMap::new();

    log::info!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Close) => {
                log::info!("Websocket Opcode::Close");
                let pid = msg.sender.pid().unwrap();
                let mut framer: Framer<rand::rngs::OsRng, embedded_websocket::Client>;
                let client = match clients.get_mut(&pid) {
                    Some(client) => match client.close() {
                        Ok(()) => log::info!("Sent close handshake"),
                        Err(e) => {
                            log::warn!("Failed to send close handshake {:?}", e);
                            xous::return_scalar(msg.sender, WsError::ProtocolError as usize).ok();
                            continue;
                        }
                    },
                    None => {
                        log::warn!("Websocket assets not in list");
                        xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                        continue;
                    }
                };

                log::info!("Websocket Opcode::Close complete");
            }
            Some(Opcode::Open) => {
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };
                let ws_config = buf.to_original::<WebsocketConfig, _>().unwrap();

                match Client::new(ws_config, ws_config.cid, ws_config.opcode) {
                    Ok(client) => {
                        let sub_protocol = client.sub_protocol;
                        clients.insert(pid, client);
                        buf.replace(Return::SubProtocol(sub_protocol))
                            .expect("failed replace buffer");
                    }
                    Err(e) => {
                        let hint = format!("failed to open Websocket {:?}", e);
                        buf.replace(drop(&hint)).expect("failed replace buffer");
                    }
                }
            }
            Some(Opcode::Poll) => {
                log::trace!("Websocket Poll");

                for (_pid, client) in &mut clients {
                    if client.empty() {
                        continue;
                    }

                    log::trace!("Websocket Read");
                    client.read();
                    log::trace!("Websocket Read complete");
                }
            }
            Some(Opcode::Send) => {
                log::info!("Websocket Opcode::Send");
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };

                let mut client = match clients.get_mut(&pid) {
                    Some(client) => client,

                    None => {
                        log::info!("Websocket assets not in list");
                        continue;
                    }
                };
                match client.write(&buf) {
                    Ok(()) => log::info!("Websocket frame sent"),
                    Err(e) => {
                        let hint = format!("failed to send Websocket frame {:?}", e);
                        buf.replace(drop(&hint)).expect("failed replace buffer");
                        continue;
                    }
                };
                log::info!("Websocket Opcode::Send complete");
            }
            Some(Opcode::State) => {
                let pid = msg.sender.pid().unwrap();
                match clients.get_mut(&pid) {
                    Some(client) => {
                        let framer = Framer::new(
                            &mut client.read_buf,
                            &mut client.read_cursor,
                            &mut client.write_buf,
                            &mut client.socket,
                        );

                        if framer.state() == WebSocketState::Open {
                            xous::return_scalar(msg.sender, 1)
                                .expect("failed to return WebSocketState");
                        }
                    }
                    None => {
                        xous::return_scalar(msg.sender, 0).expect("failed to return WebSocketState")
                    }
                };
                log::info!("Websocket Opcode::State complete");
            }
            Some(Opcode::Tick) => {
                log::info!("Websocket Opcode::Tick");
                let pid = msg.sender.pid().unwrap();
                let mut framer: Framer<rand::rngs::OsRng, embedded_websocket::Client>;
                let response = match clients.get_mut(&pid) {
                    Some(client) => {
                        framer = Framer::new(
                            &mut client.read_buf[..],
                            &mut client.read_cursor,
                            &mut client.write_buf[..],
                            &mut client.socket,
                        );
                        // TODO review keep alive request technique
                        let frame_buf = "keep alive please :-)".as_bytes();
                        framer.write(&mut client.ws_stream, MessageType::Text, true, &frame_buf)
                    }
                    None => {
                        log::warn!("Websocket assets not in list");
                        xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                        continue;
                    }
                };

                match response {
                    Ok(()) => log::info!("Websocket keep-alive request sent"),
                    Err(e) => {
                        log::info!("failed to send Websocket keep-alive request {:?}", e);
                        continue;
                    }
                };

                log::info!("Websocket Opcode::Tick complete");
            }

            Some(Opcode::Quit) => {
                log::warn!("Websocket Opcode::Quit");
                let close_op = Opcode::Close.to_usize().unwrap();
                for (_pid, client) in &mut clients {
                    xous::send_message(client.cid, xous::Message::new_scalar(close_op, 0, 0, 0, 0))
                        .expect("couldn't send Websocket poll");
                }
                log::warn!("Websocket Opcode::Quit complete");
                break;
            }
            None => {
                log::error!("couldn't convert opcode: {:?}", msg);
            }
        }
    }
    // clean up our program
    log::trace!("main loop exit, destroying servers");
    xns.unregister_server(sid).unwrap();
    xous::destroy_server(sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}

// build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
fn spawn_tick_pump(ws_manager_cid: CID) {
    thread::spawn({
        move || {
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            loop {
                tt.sleep_ms(KEEPALIVE_TIMEOUT_SECONDS.as_millis().try_into().unwrap())
                    .unwrap();
                xous::send_message(
                    ws_manager_cid,
                    xous::Message::new_scalar(
                        Opcode::Tick.to_usize().unwrap(),
                        KEEPALIVE_TIMEOUT_SECONDS.as_secs().try_into().unwrap(),
                        0,
                        0,
                        0,
                    ),
                )
                .expect("couldn't send Websocket tick");
            }
        }
    });
}

// build a thread that emits a regular WebSocketOp::Poll to send a KeepAliveRequest
fn spawn_poll_pump(ws_manager_cid: CID) {
    thread::spawn({
        move || {
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            loop {
                tt.sleep_ms(LISTENER_POLL_INTERVAL_MS.as_millis().try_into().unwrap())
                    .unwrap();
                xous::send_message(
                    ws_manager_cid,
                    xous::Message::new_scalar(
                        Opcode::Poll.to_usize().unwrap(),
                        LISTENER_POLL_INTERVAL_MS.as_millis().try_into().unwrap(),
                        0,
                        0,
                        0,
                    ),
                )
                .expect("couldn't send Websocket tick");
            }
        }
    });
}

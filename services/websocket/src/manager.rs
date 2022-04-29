#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

use super::*;
use crate::api::{validate_msg, WsError, WsStream, SUB_PROTOCOL_LEN};
use crate::poll::*;

use embedded_websocket as ws;
use num_traits::{FromPrimitive, ToPrimitive};
use rand::rngs::ThreadRng;
use rustls::{ClientConnection, StreamOwned};
use rustls_connector::*;
use std::num::NonZeroU8;
use std::{collections::HashMap, convert::TryInto, net::TcpStream, thread};
use ws::framer::{Framer, FramerError};
use ws::WebSocketCloseStatusCode as StatusCode;
use ws::WebSocketSendMessageType as MessageType;
use ws::{WebSocketClient, WebSocketOptions, WebSocketState};
use xous::{CID, SID};
use xous_ipc::Buffer;

use url::Url;

use std::io::{Read, Write};
use std::time::Duration;

/** time between reglar websocket keep-alive requests */
pub(crate) const KEEPALIVE_TIMEOUT_SECONDS: Duration = Duration::from_secs(55);
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

pub(crate) struct Client<R: rand::RngCore> {
    /** the configuration of an open websocket */
    socket: WebSocketClient<R>,
    /** a websocket stream when opened on a tls connection */
    wss_stream: Option<WsStream<StreamOwned<ClientConnection, TcpStream>>>,
    /** a websocket stream when opened on a tcp connection */
    ws_stream: Option<WsStream<TcpStream>>,
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
}

impl<'a, R: rand::RngCore> Client<R> {
    pub(crate) fn new(ws_config: WebsocketConfig) -> Self {
        // construct url from ws_config
        let host = ws_config.host.as_str().expect("url utf-8 decode fail");
        let mut url = Url::parse(host).expect("invalid host");
        let path = match ws_config.path {
            Some(path) => {
                let path = path.unwrap().as_str().expect("path utf-8 decode error");
                url = url.join(path).expect("valid path");
            }
            None => "",
        };
        if ws_config.port.is_some() {
            url.set_port(
                ws_config
                    .port
                    .unwrap()
                    .try_into()
                    .expect("login utf-8 decode error"),
            );
        }
        if ws_config.login.is_some() {
            let login = ws_config
                .login
                .unwrap()
                .as_str()
                .expect("login utf-8 decode error");
            url.query_pairs_mut().append_pair("login", &login);
        }
        if ws_config.password.is_some() {
            let password = ws_config
                .password
                .unwrap()
                .as_str()
                .expect("password utf-8 decode error");
            url.query_pairs_mut().append_pair("password", &password);
        }
        match ws_config.certificate_authority.is_some() {
            true => url.set_scheme("wss").expect("fail set url scheme"),
            false => url.set_scheme("ws").expect("fail set url scheme"),
        };

        // Create a TCP Stream between this device and the remote Server
        log::info!("Opening TCP connection to {:?}", host);
        let tcp_stream = match TcpStream::connect(url.as_str()) {
            Ok(tcp_stream) => tcp_stream,
            Err(e) => log::warn!("Failed to open TCP Stream {:?}", e),
        };

        log::info!("TCP connected to {:?}", host);

        let mut read_buf = [0u8; WEBSOCKET_BUFFER_LEN];
        let mut read_cursor = 0;
        let mut write_buf = [0u8; WEBSOCKET_BUFFER_LEN];
        let mut socket = WebSocketClient::new_client(rand::thread_rng());
        let mut framer = Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, &mut socket);

        let websocket_options = WebSocketOptions {
            path: path,
            host: host,
            origin: &url.origin().unicode_serialization(),
            sub_protocols: [
                ws_config.sub_protocols[0].as_str().unwrap(),
                ws_config.sub_protocols[1].as_str().unwrap(),
                ws_config.sub_protocols[2].as_str().unwrap(),
            ],
            additional_headers: None,
        };

        let ws_stream = None;
        let wss_stream = None;
        let tcp_clone = match tcp_stream.try_clone() {
            Ok(c) => c,
            Err(e) => log::warn!("Failed to clone TCP Stream {:?}", e),
        };
        let sub_protocol: xous_ipc::String<{ SUB_PROTOCOL_LEN }>;
        if ws_config.certificate_authority.is_none() {
            // Initiate a websocket opening handshake over the TCP Stream
            let stream = WsStream(tcp_stream);
            sub_protocol = match framer.connect(&mut stream, &websocket_options) {
                Ok(opt) => match opt {
                    Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                    None => xous_ipc::String::from_str(""),
                },
                Err(e) => log::warn!("Unable to connect WebSocket {:?}", e),
            };
            ws_stream = Some(stream);
        } else {
            // Create a TLS connection to the remote Server on the TCP Stream
            let ca = ws_config
                .certificate_authority
                .unwrap()
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
            let tls_stream = match tls_connector.connect(url.host_str().unwrap(), tcp_stream) {
                Ok(tls_stream) => {
                    log::info!("TLS connected to {:?}", url.host_str().unwrap());
                    tls_stream
                }
                Err(e) => log::warn!("Failed to complete TLS handshake {:?}", e),
            };
            // Initiate a websocket opening handshake over the TLS Stream
            let stream = WsStream(tls_stream);
            sub_protocol = match framer.connect(&mut stream, &websocket_options) {
                Ok(opt) => match opt {
                    Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                    None => xous_ipc::String::from_str(""),
                },
                Err(e) => log::warn!("Unable to connect WebSocket {:?}", e),
            };
            wss_stream = Some(stream);
        }
        log::info!("WebSocket connected with protocol: {:?}", sub_protocol);

        Client {
            socket,
            wss_stream,
            ws_stream,
            tcp_clone,
            read_buf,
            read_cursor,
            write_buf,
        }
    }

    pub(crate) fn Ok(&self) -> bool {
        self.framer.status() == WebSocketState::Open;
    }

    pub(crate) fn sub_protocol(&self) -> Option<&str> {
        //let mut response = api::Return::SubProtocol(sub_protocol);
        self.sub_protocol
    }

    fn spawn_poll(&self, cid: CID) {
        let mut poll = Poll::new(
            self.cid,
            self.opcode,
            self.tcp_clone,
            self.ws_stream,
            self.wss_stream,
            self.socket,
        );

        thread::spawn({
            move || {
                poll.main(cid);
            }
        });
    }

    fn write<E, T: Read + Write>(&self, buffer: &[u8]) -> Result<(), FramerError<E>> {
        let mut ret = Ok(());

        let mut framer = Framer::new(
            &mut self.read_buf[..],
            &mut self.read_cursor,
            &mut self.write_buf[..],
            &mut self.socket,
        );

        let stream = match self.wss_stream {
            Some(stream) => stream,
            None => match self.ws_stream {
                Some(stream) => stream,
                None => {
                    log::warn!("Assets missing both wss_stream and ws_stream");
                    return Err();
                }
            },
        };

        match framer.state() {
            WebSocketState::Open => {
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
                    ret = framer.write(&mut *stream, MessageType::Binary, end_of_message, slice);
                    start = start + WEBSOCKET_PAYLOAD_LEN;
                }
                ret
            }
            _ => {
                let hint = format!("WebSocket DOA {:?}", framer.state());
                buf.replace(drop(&hint)).expect("failed replace buffer");
                Err()
            }
        }
    }

    fn close(&self) -> Result<(), FramerError<E>> {
        let mut ret = Ok(());

        let mut framer = Framer::new(
            &mut self.read_buf[..],
            &mut self.read_cursor,
            &mut self.write_buf[..],
            &mut self.socket,
        );

        let stream = match self.wss_stream {
            Some(stream) => stream,
            None => match self.ws_stream {
                Some(stream) => stream,
                None => {
                    log::warn!("Assets missing both wss_stream and ws_stream");
                    return Err();
                }
            },
        };

        framer.close(&mut *stream, StatusCode::NormalClosure, None)
    }
}

pub(crate) fn main(sid: SID) -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let cid = xous::connect(sid).unwrap();

    // build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
    spawn_tick_pump(cid);

    /* holds the assets of existing websockets by pid - and as such - limits each pid to 1 websocket. */
    // TODO review the limitation of 1 websocket per pid.
    let mut clients: HashMap<NonZeroU8, Client<ThreadRng>> = HashMap::new();

    log::trace!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Close) => {
                log::info!("Websocket Opcode::Close");
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Close.to_u32().unwrap()) {
                    continue;
                }
                let pid = msg.sender.pid().unwrap();
                let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
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
                if !validate_msg(&mut msg, WsError::Memory, Opcode::Open.to_u32().unwrap()) {
                    continue;
                }
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };
                let ws_config = buf.to_original::<WebsocketConfig, _>().unwrap();
                let ws_client = Client::new(ws_config);
                clients.insert(pid, ws_client);
                ws_client.spawn_poll(cid);
            }
            Some(Opcode::Send) => {
                if !validate_msg(&mut msg, WsError::Memory, Opcode::Send.to_u32().unwrap()) {
                    continue;
                }
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
                log::info!("Websocket Opcode::State");
                if !validate_msg(
                    &mut msg,
                    WsError::ScalarBlock,
                    Opcode::State.to_u32().unwrap(),
                ) {
                    continue;
                }
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
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Tick.to_u32().unwrap()) {
                    continue;
                }
                let pid = msg.sender.pid().unwrap();
                let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
                let (wss_stream, ws_stream) = match clients.get_mut(&pid) {
                    Some(client) => {
                        framer = Framer::new(
                            &mut client.read_buf[..],
                            &mut client.read_cursor,
                            &mut client.write_buf[..],
                            &mut client.socket,
                        );
                        (&mut client.wss_stream, &mut client.ws_stream)
                    }
                    None => {
                        log::warn!("Websocket assets not in list");
                        xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                        continue;
                    }
                };

                // TODO review keep alive request technique
                let frame_buf = "keep alive please :-)".as_bytes();

                let response = match wss_stream {
                    Some(stream) => framer.write(&mut *stream, MessageType::Text, true, &frame_buf),

                    None => match ws_stream {
                        Some(stream) => {
                            framer.write(&mut *stream, MessageType::Text, true, &frame_buf)
                        }

                        None => {
                            log::warn!("Assets missing both wss_stream and ws_stream");
                            xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                            continue;
                        }
                    },
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
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Quit.to_u32().unwrap()) {
                    continue;
                }
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

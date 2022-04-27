#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
use api::*;
mod client;
use client::*;

use derive_deref::*;
use embedded_websocket as ws;
use num_traits::{FromPrimitive, ToPrimitive};
use rand::rngs::ThreadRng;
use rustls::{ClientConnection, StreamOwned};
use rustls_connector::*;
use std::num::NonZeroU8;
use std::{
    collections::HashMap,
    convert::TryInto,
    io::{Error, ErrorKind, Read, Write},
    net::TcpStream,
    thread,
};
use url::Url;
use ws::framer::{Framer, FramerError};
use ws::WebSocketCloseStatusCode as StatusCode;
use ws::WebSocketSendMessageType as MessageType;
use ws::{WebSocketClient, WebSocketOptions, WebSocketState};
use xous::CID;
use xous_ipc::Buffer;

#[derive(Clone, Copy, Debug, Deref, DerefMut)]
struct WsStream<T: Read + Write>(T);

impl<T: Read + Write> ws::framer::Stream<Error> for WsStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, Error> {
        self.0.read(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> std::result::Result<(), Error> {
        self.0.write_all(buf)
    }
}

/** Holds the machinery for each websocket between Opcode calls */
struct Assets<R: rand::RngCore> {
    //, T:Read + Write> {
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

impl<R: rand::RngCore> Assets<R> {
    fn framer(&mut self) -> Framer<R, embedded_websocket::Client> {
        Framer::new(
            &mut self.read_buf[..],
            &mut self.read_cursor,
            &mut self.write_buf[..],
            &mut self.socket,
        )
    }
    /*
    fn stream(&mut self) -> Option<WsStream<T>> {
        match self.wss_stream {
            Some(stream) => Some(stream),
            None => match self.ws_stream {
                Some(stream) => Some(stream),
                None => None,
            },
        }
    }
    */
}

#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let ws_sid = xns
        .register_name(api::SERVER_NAME_WEBSOCKET, None)
        .expect("can't register server");
    log::trace!("registered with NS -- {:?}", ws_sid);
    let ws_cid = xous::connect(ws_sid).unwrap();

    // build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
    spawn_tick_pump(ws_cid);
    // build a thread that emits a regular WebSocketOp::Poll to check for inbound websocket frames
    spawn_poll_pump(ws_cid);

    /* holds the assets of existing websockets by pid - and as such - limits each pid to 1 websocket. */
    // TODO review the limitation of 1 websocket per pid.
    let mut store: HashMap<NonZeroU8, Assets<ThreadRng>> = HashMap::new();

    log::trace!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(ws_sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Close) => {
                log::info!("Websocket Opcode::Close");
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Close) {
                    continue;
                }
                let pid = msg.sender.pid().unwrap();
                let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
                let (wss_stream, ws_stream) = match store.get_mut(&pid) {
                    Some(assets) => {
                        framer = Framer::new(
                            &mut assets.read_buf[..],
                            &mut assets.read_cursor,
                            &mut assets.write_buf[..],
                            &mut assets.socket,
                        );
                        (&mut assets.wss_stream, &mut assets.ws_stream)
                    }
                    None => {
                        log::warn!("Websocket assets not in list");
                        xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                        continue;
                    }
                };

                let response = match wss_stream {
                    Some(stream) => framer.close(&mut *stream, StatusCode::NormalClosure, None),
                    None => match ws_stream {
                        Some(stream) => framer.close(&mut *stream, StatusCode::NormalClosure, None),
                        None => {
                            log::warn!("Assets missing both wss_stream and ws_stream");
                            xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                            continue;
                        }
                    },
                };

                match response {
                    Ok(()) => log::info!("Sent close handshake"),
                    Err(e) => {
                        log::warn!("Failed to send close handshake {:?}", e);
                        xous::return_scalar(msg.sender, WsError::ProtocolError as usize).ok();
                        continue;
                    }
                };
                log::info!("Websocket Opcode::Close complete");
            }
            Some(Opcode::Open) => {
                log::info!("Websocket Opcode::Open");
                if !validate_msg(&mut msg, WsError::MemoryBlock, Opcode::Open) {
                    continue;
                }
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };
                if store.contains_key(&pid) {
                    buf.replace(drop("WebSocket already open"))
                        .expect("failed replace buffer");
                    continue;
                }
                let ws_config = buf.to_original::<WebsocketConfig, _>().unwrap();

                // construct url from ws_config
                let url = ws_config.base_url.as_str().expect("url utf-8 decode fail");
                let path = ws_config.path.as_str().expect("path utf-8 decode error");
                let mut url = Url::parse(url).expect("invalid base_url");
                url = url.join(path).expect("valid path");
                match ws_config.certificate_authority.is_some() {
                    true => url.set_scheme("wss").expect("fail set url scheme"),
                    false => url.set_scheme("ws").expect("fail set url scheme"),
                };
                if ws_config.use_credentials {
                    let login = ws_config.login.as_str().expect("login utf-8 decode error");
                    let password = ws_config
                        .password
                        .as_str()
                        .expect("password utf-8 decode error");
                    url.query_pairs_mut()
                        .append_pair("login", &login)
                        .append_pair("password", &password);
                }
                let protocols = [
                    ws_config.sub_protocols[0].as_str().unwrap(),
                    ws_config.sub_protocols[1].as_str().unwrap(),
                    ws_config.sub_protocols[2].as_str().unwrap(),
                ];
                let websocket_options = WebSocketOptions {
                    path: &path,
                    host: &url.host_str().unwrap(),
                    origin: &url.origin().unicode_serialization(),
                    sub_protocols: Some(&protocols),
                    additional_headers: None,
                };
                let mut read_buf = [0; WEBSOCKET_BUFFER_LEN];
                let mut read_cursor = 0;
                let mut write_buf = [0; WEBSOCKET_BUFFER_LEN];

                let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
                let mut framer = Framer::new(
                    &mut read_buf,
                    &mut read_cursor,
                    &mut write_buf,
                    &mut ws_client,
                );

                log::trace!("Will start websocket at {:?}", url.host_str().unwrap());
                // Create a TCP Stream between this device and the remote Server
                let target = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());
                log::info!("Opening TCP connection to {:?}", target);
                let tcp_stream = match TcpStream::connect(&target) {
                    Ok(tcp_stream) => tcp_stream,
                    Err(e) => {
                        let hint = format!("Failed to open TCP Stream {:?}", e);
                        buf.replace(drop(&hint)).expect("failed replace buffer");
                        continue;
                    }
                };

                log::info!("TCP connected to {:?}", target);

                let mut ws_stream = None;
                let mut wss_stream = None;
                let tcp_clone = match tcp_stream.try_clone() {
                    Ok(c) => c,
                    Err(e) => {
                        let hint = format!("Failed to clone TCP Stream {:?}", e);
                        buf.replace(drop(&hint)).expect("failed replace buffer");
                        continue;
                    }
                };
                let sub_protocol: xous_ipc::String<SUB_PROTOCOL_LEN>;
                if ws_config.certificate_authority.is_none() {
                    // Initiate a websocket opening handshake over the TCP Stream
                    let mut stream = WsStream(tcp_stream);
                    sub_protocol = match framer.connect(&mut stream, &websocket_options) {
                        Ok(opt) => match opt {
                            Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                            None => xous_ipc::String::from_str(""),
                        },
                        Err(e) => {
                            let hint = format!("Unable to connect WebSocket {:?}", e);
                            buf.replace(drop(&hint)).expect("failed replace buffer");
                            continue;
                        }
                    };
                    ws_stream = Some(stream);
                } else {
                    // Create a TLS connection to the remote Server on the TCP Stream
                    let ca = ws_config.certificate_authority.unwrap();
                    let ca = ca
                        .as_str()
                        .expect("certificate_authority utf-8 decode error");
                    let tls_connector = RustlsConnector::from(ssl_config(ca));
                    let tls_stream =
                        match tls_connector.connect(url.host_str().unwrap(), tcp_stream) {
                            Ok(tls_stream) => {
                                log::info!("TLS connected to {:?}", url.host_str().unwrap());
                                tls_stream
                            }
                            Err(e) => {
                                let hint = format!("Failed to complete TLS handshake {:?}", e);
                                buf.replace(drop(&hint)).expect("failed replace buffer");
                                continue;
                            }
                        };
                    // Initiate a websocket opening handshake over the TLS Stream
                    let mut stream = WsStream(tls_stream);
                    sub_protocol = match framer.connect(&mut stream, &websocket_options) {
                        Ok(opt) => match opt {
                            Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                            None => xous_ipc::String::from_str(""),
                        },
                        Err(e) => {
                            let hint = format!("Unable to connect WebSocket {:?}", e);
                            buf.replace(drop(&hint)).expect("failed replace buffer");
                            continue;
                        }
                    };
                    wss_stream = Some(stream);
                }

                let mut response = api::Return::SubProtocol(sub_protocol);
                match framer.state() {
                    WebSocketState::Open => {
                        log::info!("WebSocket connected with protocol: {:?}", sub_protocol);

                        // Store the open websocket indexed by the calling pid
                        store.insert(
                            pid,
                            Assets {
                                socket: ws_client,
                                wss_stream: wss_stream,
                                ws_stream: ws_stream,
                                tcp_stream: tcp_clone,
                                read_buf: read_buf,
                                read_cursor: read_cursor,
                                write_buf: write_buf,
                                cid: ws_config.cid,
                                opcode: ws_config.opcode,
                            },
                        );
                    }
                    _ => {
                        let hint = format!("WebSocket failed to connect {:?}", framer.state());
                        response = drop(&hint);
                    }
                }

                buf.replace(response).expect("failed replace buffer");
                log::info!("Websocket Opcode::Open complete");
            }
            Some(Opcode::Poll) => {
                log::trace!("Websocket Opcode::Poll");
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Poll) {
                    continue;
                }
                // Check each websocket for an inbound frame to read and send to the cid

                for (pid, assets) in &mut store {
                    log::trace!("Websocket Opcode::Poll PID={:?}", pid);

                    if empty(&mut assets.tcp_stream) {
                        continue;
                    }

                    let mut framer = Framer::new(
                        &mut assets.read_buf,
                        &mut assets.read_cursor,
                        &mut assets.write_buf,
                        &mut assets.socket,
                    );

                    let (wss_stream, ws_stream, cid, opcode) = {
                        (
                            &mut assets.wss_stream,
                            &mut assets.ws_stream,
                            assets.cid,
                            assets.opcode,
                        )
                    };

                    match wss_stream {
                        Some(stream) => read(&mut framer, &mut *stream, cid, opcode),
                        None => match ws_stream {
                            Some(stream) => read(&mut framer, &mut *stream, cid, opcode),

                            None => {
                                log::warn!("Assets missing both wss_stream and ws_stream");
                                xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                                continue;
                            }
                        },
                    };
                }
                log::trace!("Websocket Opcode::Poll complete");
            }
            Some(Opcode::Send) => {
                if !validate_msg(&mut msg, WsError::Memory, Opcode::Send) {
                    continue;
                }
                log::info!("Websocket Opcode::Send");
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };

                let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
                let (wss_stream, ws_stream) = match store.get_mut(&pid) {
                    Some(assets) => {
                        framer = Framer::new(
                            &mut assets.read_buf[..],
                            &mut assets.read_cursor,
                            &mut assets.write_buf[..],
                            &mut assets.socket,
                        );
                        (&mut assets.wss_stream, &mut assets.ws_stream)
                    }
                    None => {
                        log::info!("Websocket assets not in list");
                        continue;
                    }
                };
                match framer.state() {
                    WebSocketState::Open => {}
                    _ => {
                        let hint = format!("WebSocket DOA {:?}", framer.state());
                        buf.replace(drop(&hint)).expect("failed replace buffer");
                        continue;
                    }
                }

                let response = match wss_stream {
                    Some(stream) => write(&mut framer, &mut *stream, &buf),
                    None => match ws_stream {
                        Some(stream) => write(&mut framer, &mut *stream, &buf),
                        None => {
                            log::warn!("Assets missing both wss_stream and ws_stream");
                            continue;
                        }
                    },
                };
                match response {
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
                if !validate_msg(&mut msg, WsError::ScalarBlock, Opcode::State) {
                    continue;
                }
                let pid = msg.sender.pid().unwrap();
                match store.get_mut(&pid) {
                    Some(assets) => {
                        let framer = Framer::new(
                            &mut assets.read_buf,
                            &mut assets.read_cursor,
                            &mut assets.write_buf,
                            &mut assets.socket,
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
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Tick) {
                    continue;
                }
                let pid = msg.sender.pid().unwrap();
                let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
                let (wss_stream, ws_stream) = match store.get_mut(&pid) {
                    Some(assets) => {
                        framer = Framer::new(
                            &mut assets.read_buf[..],
                            &mut assets.read_cursor,
                            &mut assets.write_buf[..],
                            &mut assets.socket,
                        );
                        (&mut assets.wss_stream, &mut assets.ws_stream)
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
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Quit) {
                    continue;
                }
                let close_op = Opcode::Close.to_usize().unwrap();
                for (_pid, assets) in &mut store {
                    xous::send_message(assets.cid, xous::Message::new_scalar(close_op, 0, 0, 0, 0))
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
    xns.unregister_server(ws_sid).unwrap();
    xous::destroy_server(ws_sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}

// build a thread that emits a regular WebSocketOp::Poll to check for inbound websocket frames
fn spawn_poll_pump(cid: CID) {
    thread::spawn({
        move || {
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            loop {
                tt.sleep_ms(LISTENER_POLL_INTERVAL_MS.as_millis().try_into().unwrap())
                    .unwrap();
                xous::send_message(
                    cid,
                    xous::Message::new_scalar(
                        Opcode::Poll.to_usize().unwrap(),
                        LISTENER_POLL_INTERVAL_MS.as_secs().try_into().unwrap(),
                        0,
                        0,
                        0,
                    ),
                )
                .expect("couldn't send Websocket poll");
            }
        }
    });
}

// build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
fn spawn_tick_pump(cid: CID) {
    thread::spawn({
        move || {
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            loop {
                tt.sleep_ms(KEEPALIVE_TIMEOUT_SECONDS.as_millis().try_into().unwrap())
                    .unwrap();
                xous::send_message(
                    cid,
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

/** helper function to return hints from opcode panics */
fn drop(hint: &str) -> api::Return {
    log::warn!("{}", hint);
    api::Return::Failure(xous_ipc::String::from_str(hint))
}

fn empty(stream: &mut TcpStream) -> bool {
    stream
        .set_nonblocking(true)
        .expect("failed to set TCP Stream to non-blocking");
    let mut frame_buf = [0u8; 8];
    let empty = match stream.peek(&mut frame_buf) {
        Ok(_) => false,
        Err(ref e) if e.kind() == ErrorKind::WouldBlock => true,
        Err(e) => {
            log::warn!("TCP IO error: {}", e);
            true
        }
    };
    stream
        .set_nonblocking(false)
        .expect("failed to set TCP Stream to non-blocking");
    empty
}

/** read all available frames from the websocket and relay each frame to the caller_id */
fn read<E, R, S, T>(framer: &mut Framer<R, S>, stream: &mut T, cid: CID, opcode: u32)
where
    E: std::fmt::Debug,
    R: rand::RngCore,
    T: ws::framer::Stream<E>,
    S: ws::WebSocketType,
{
    let mut frame_buf = [0u8; WEBSOCKET_PAYLOAD_LEN];
    while let Some(frame) = framer
        .read_binary(&mut *stream, &mut frame_buf[..])
        .expect("failed to read websocket")
    {
        let frame: [u8; WEBSOCKET_PAYLOAD_LEN] = frame
            .try_into()
            .expect("websocket frame too large for buffer");
        let buf = Buffer::into_buf(Frame { bytes: frame })
            .expect("failed to serialize websocket frame into buffer");
        buf.send(cid, opcode)
            .expect("failed to relay websocket frame");
    }
}

fn write<E, R, S, T>(framer: &mut Framer<R, S>, stream: &mut T, buffer: &[u8]) -> Result<(), FramerError<E>>
where
    E: std::fmt::Debug,
    R: rand::RngCore,
    T: ws::framer::Stream<E>,
    S: ws::WebSocketType,
{
    let mut ret = Ok(());
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
    };
    ret
}

/** complete the machinations of setting up a rustls::ClientConfig */
fn ssl_config(certificate_authority: &str) -> rustls::ClientConfig {
    let mut cert_bytes = std::io::Cursor::new(&certificate_authority);
    let roots = rustls_pemfile::certs(&mut cert_bytes).expect("parseable PEM files");
    let roots = roots.iter().map(|v| rustls::Certificate(v.clone()));

    let mut root_certs = rustls::RootCertStore::empty();
    for root in roots {
        root_certs.add(&root).unwrap();
    }

    rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_certs)
        .with_no_client_auth()
}

fn validate_msg(env: &mut xous::MessageEnvelope, expected: WsError, opcode: Opcode) -> bool {
    let is_blocking = env.body.is_blocking();
    match env.body.memory_message_mut() {
        None => {
            if (expected == WsError::Scalar && is_blocking)
                || (expected == WsError::ScalarBlock && !is_blocking)
            {
                log::warn!("invalid xous:MessageEnvelope for Opcode::{:#?}", opcode);
                xous::return_scalar(env.sender, expected as usize).ok();
                return false;
            };
        }
        Some(body) => {
            if (expected == WsError::Memory && is_blocking)
                || (expected == WsError::MemoryBlock && !is_blocking)
            {
                log::warn!("invalid xous:MessageEnvelope for Opcode::{:#?}", opcode);
                body.valid = None;
                let s: &mut [u8] = body.buf.as_slice_mut();
                let mut i = s.iter_mut();

                // Duplicate error to ensure it's seen as an error regardless of byte order/return type
                // This is necessary because errors are encoded as `u8` slices, but "good"
                // responses may be encoded as `u16` or `u32` slices.
                *i.next().expect("failed to set msg byte") = 1;
                *i.next().expect("failed to set msg byte") = 1;
                *i.next().expect("failed to set msg byte") = 1;
                *i.next().expect("failed to set msg byte") = 1;
                *i.next().expect("failed to set msg byte") = expected as u8;
                *i.next().expect("failed to set msg byte") = 0;
                *i.next().expect("failed to set msg byte") = 0;
                *i.next().expect("failed to set msg byte") = 0;
                return false;
            }
        }
    };
    true
}

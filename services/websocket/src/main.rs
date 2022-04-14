#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
use api::*;

mod ws_test;

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
    io::{Error, Read, Write},
    net::TcpStream,
    thread,
};
use url::Url;
use ws::framer::Framer;
use ws::WebSocketCloseStatusCode as StatusCode;
use ws::WebSocketSendMessageType as MessageType;
use ws::{WebSocketClient, WebSocketOptions};
use xous::CID;
use xous_ipc::Buffer;

#[derive(Deref, DerefMut)]
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
    /** the configuration of an open websocket */
    socket: WebSocketClient<R>,
    /** a websocket stream when opened on a tls conneciton */
    wss_stream: Option<WsStream<StreamOwned<ClientConnection, TcpStream>>>,
    /** a websocket stream when opened on a tcp conneciton */
    ws_stream: Option<WsStream<TcpStream>>,
    /** the callback_id to use when relaying an inbound websocket frame */
    cid: CID,
    /** the opcode to use when relaying an inbound websocket frame */
    opcode: u32,
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

    #[cfg(feature = "ws_test")]
    /** Run a basic test on a tcp & tls connection to a local websocket server listening on 127.0.0.1:1337. */
    {
        let test_op = Opcode::Test.to_usize().unwrap();
        xous::send_message(ws_cid, xous::Message::new_scalar(test_op, 0, 0, 0, 0))
            .expect("failed to send Opcode::Test");
    }

    // build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
    spawn_tick_pump(ws_cid);
    // build a thread that emits a regular WebSocketOp::Poll to check for inbound websocket frames
    spawn_poll_pump(ws_cid);

    let mut read_buf = [0; WEBSOCKET_BUFFER_LEN];
    let mut read_cursor = 0;
    let mut write_buf = [0; WEBSOCKET_BUFFER_LEN];
    let mut frame_buf = [0; WEBSOCKET_BUFFER_LEN];
    let mut _read_buf = [0; WEBSOCKET_BUFFER_LEN];

    /** holds the assets of existing websockets by pid - and as such - limits each pid to 1 websocket. */
    // TODO review the limitation of 1 websocket per pid.
    let mut store: HashMap<NonZeroU8, Assets<ThreadRng>> = HashMap::new();

    log::trace!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(ws_sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Close) => {
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };
                let (mut socket, wss_stream, ws_stream) = match store.get_mut(&pid) {
                    Some(assets) => (
                        &mut assets.socket,
                        &mut assets.wss_stream,
                        &mut assets.ws_stream,
                    ),
                    None => {
                        buf.replace(drop("Websocket assets not in list")).unwrap();
                        continue;
                    }
                };
                zero(&mut vec![&mut read_buf[..], &mut write_buf[..]]);
                let mut framer =
                    Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, &mut socket);

                let response = match wss_stream {
                    Some(stream) => framer.close(&mut *stream, StatusCode::NormalClosure, None),

                    None => match ws_stream {
                        Some(stream) => framer.close(&mut *stream, StatusCode::NormalClosure, None),

                        None => {
                            log::info!("Assets missing both wss_stream and ws_stream");
                            continue;
                        }
                    },
                };
                match response {
                    Ok(()) => log::info!("Sent close handshake"),
                    Err(e) => {
                        let hint = format!("Failed to send close handshake {:?}", e);
                        buf.replace(drop(&hint)).unwrap();
                        continue;
                    }
                };
            }
            Some(Opcode::Open) => {
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };
                if store.contains_key(&pid) {
                    buf.replace(drop("WebSocket already open")).unwrap();
                    continue;
                }
                let ws_config = buf.to_original::<WebsocketConfig, _>().unwrap();
                let base_url = ws_config
                    .base_url
                    .as_str()
                    .expect("base_url utf-8 decode error");
                let mut url = Url::parse(base_url).expect("valid base_url");
                let path = ws_config.path.as_str().expect("path utf-8 decode error");
                url = url.join(path).expect("valid path");
                url.set_scheme("wss").expect("valid https base url");

                log::trace!("Will start websocket at {:?}", url.as_str());

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
                let websocket_options = WebSocketOptions {
                    path: &path,
                    host: &base_url,
                    origin: "",
                    sub_protocols: None,
                    additional_headers: None,
                };
                let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
                zero(&mut vec![&mut read_buf[..], &mut write_buf[..]]);
                let mut framer = Framer::new(
                    &mut read_buf,
                    &mut read_cursor,
                    &mut write_buf,
                    &mut ws_client,
                );

                // Create a TCP Stream between this device and the remote Server
                let tcp_stream = match TcpStream::connect(url.as_str()) {
                    Ok(tcp_stream) => tcp_stream,
                    Err(e) => {
                        let hint = format!("Failed to open TCP Stream {:?}", e);
                        buf.replace(drop(&hint)).unwrap();
                        continue;
                    }
                };
                log::info!("TCP connected to {:?}", base_url);

                let ws_stream = None;
                let wss_stream = None;
                let sub_protocol: xous_ipc::String<HINT_LEN>;
                if ws_config.certificate_authority.is_none() {
                    // Initiate a websocket opening handshake over the TCP Stream
                    let ws_stream = Some(WsStream(tcp_stream));
                    sub_protocol = match framer.connect(&mut ws_stream.unwrap(), &websocket_options)
                    {
                        Ok(opt) => match opt {
                            Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                            None => xous_ipc::String::from_str(""),
                        },
                        Err(e) => {
                            let hint = format!("Unable to connect WebSocket {:?}", e);
                            buf.replace(drop(&hint)).unwrap();
                            continue;
                        }
                    };
                } else {
                    // Create a TLS connection to the remote Server on the TCP Stream
                    let ca = ws_config.certificate_authority.unwrap();
                    let ca = ca
                        .as_str()
                        .expect("certificate_authority utf-8 decode error");
                    let tls_connector = RustlsConnector::from(ssl_config(ca));
                    let tls_stream = match tls_connector.connect(base_url, tcp_stream) {
                        Ok(tls_stream) => {
                            log::info!("TLS connected to {:?}", base_url);
                            tls_stream
                        }
                        Err(e) => {
                            let hint = format!("Failed to complete TLS handshake {:?}", e);
                            buf.replace(drop(&hint)).unwrap();
                            continue;
                        }
                    };
                    // Initiate a websocket opening handshake over the TLS Stream
                    let wss_stream = Some(WsStream(tls_stream));
                    sub_protocol =
                        match framer.connect(&mut wss_stream.unwrap(), &websocket_options) {
                            Ok(opt) => match opt {
                                Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                                None => xous_ipc::String::from_str(""),
                            },
                            Err(e) => {
                                let hint = format!("Unable to connect WebSocket {:?}", e);
                                buf.replace(drop(&hint)).unwrap();
                                continue;
                            }
                        };
                }

                log::info!("WebSocket connected with: {:?}", sub_protocol);

                // Store the open websocket indexed by the calling pid
                store.insert(
                    pid,
                    Assets {
                        socket: ws_client,
                        wss_stream: wss_stream,
                        ws_stream: ws_stream,
                        cid: ws_config.cid,
                        opcode: ws_config.opcode,
                    },
                );

                let response = api::Return::SubProtocol(sub_protocol);
                buf.replace(response).unwrap();
            }
            Some(Opcode::Poll) => {
                // Check each websocket for an inbound frame to read and send to the cid
                for (_pid, assets) in &mut store {
                    zero(&mut vec![&mut read_buf[..], &mut write_buf[..]]);
                    let mut framer = Framer::new(
                        &mut read_buf,
                        &mut read_cursor,
                        &mut write_buf,
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
                        Some(stream) => {
                            poll(&mut framer, &mut *stream, &mut frame_buf, cid, opcode)
                        }

                        None => match ws_stream {
                            Some(stream) => {
                                poll(&mut framer, &mut *stream, &mut frame_buf, cid, opcode)
                            }

                            None => {
                                log::info!("Assets missing both wss_stream and ws_stream");
                                continue;
                            }
                        },
                    };
                }

                log::info!("Websocket poll complete");
            }
            Some(Opcode::Send) => xous::msg_scalar_unpack!(msg, msg_type, _, _, _, {
                let pid = msg.sender.pid().unwrap();
                let mut buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };
                let (socket, wss_stream, ws_stream) = match store.get_mut(&pid) {
                    Some(assets) => (
                        &mut assets.socket,
                        &mut assets.wss_stream,
                        &mut assets.ws_stream,
                    ),
                    None => {
                        buf.replace(drop("Websocket assets not in list")).unwrap();
                        continue;
                    }
                };
                let ws_msg_type = match FromPrimitive::from_usize(msg_type) {
                    Some(SendMessageType::Text) => MessageType::Text,
                    Some(SendMessageType::Binary) => MessageType::Binary,
                    invalid => {
                        let hint = format!("Invalid value SendMessageType: {:?}", invalid);
                        buf.replace(drop(&hint)).unwrap();
                        continue;
                    }
                };

                zero(&mut vec![&mut read_buf[..], &mut write_buf[..]]);
                let mut framer =
                    Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, socket);

                let response = match wss_stream {
                    Some(stream) => framer.write(&mut *stream, ws_msg_type, true, &buf),

                    None => match ws_stream {
                        Some(stream) => framer.write(&mut *stream, ws_msg_type, true, &buf),

                        None => {
                            log::info!("Assets missing both wss_stream and ws_stream");
                            continue;
                        }
                    },
                };
                match response {
                    Ok(()) => log::info!("Websocket frame sent"),
                    Err(e) => {
                        let hint = format!("failed to send Websocket frame {:?}", e);
                        buf.replace(drop(&hint)).unwrap();
                        continue;
                    }
                };
            }),
            #[cfg(feature = "ws_test")]
            Some(Opcode::Test) => {
                let _pid = msg.sender.pid().unwrap();
                ws_test::local::tcp();
                // ws_test::local::tls();
            }
            Some(Opcode::Tick) => {
                let pid = msg.sender.pid().unwrap();
                let (socket, wss_stream, ws_stream) = match store.get_mut(&pid) {
                    Some(assets) => (
                        &mut assets.socket,
                        &mut assets.wss_stream,
                        &mut assets.ws_stream,
                    ),
                    None => {
                        log::info!("Websocket assets not in list");
                        continue;
                    }
                };
                // TODO review keep alive request technique
                let frame_buf = "keep alive please :-)".as_bytes();

                zero(&mut vec![&mut read_buf[..], &mut write_buf[..]]);
                let mut framer =
                    Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, socket);

                let response = match wss_stream {
                    Some(stream) => framer.write(&mut *stream, MessageType::Text, true, &frame_buf),

                    None => match ws_stream {
                        Some(stream) => {
                            framer.write(&mut *stream, MessageType::Text, true, &frame_buf)
                        }

                        None => {
                            log::info!("Assets missing both wss_stream and ws_stream");
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

                log::info!("Websocket keep-alive request sent");
            }

            Some(Opcode::Quit) => {
                log::warn!("got quit!");
                let close_op = Opcode::Close.to_usize().unwrap();
                for (_pid, assets) in &mut store {
                    xous::send_message(ws_cid, xous::Message::new_scalar(close_op, 0, 0, 0, 0))
                        .expect("couldn't send Websocket poll");
                }
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
    log::info!("{}", hint);
    api::Return::Failure(xous_ipc::String::from_str(hint))
}

/** fill all Arrays in the Vector with 0's  */
fn zero(dirty: &mut Vec<&mut [u8]>) {
    dirty
        .iter_mut()
        .for_each(|d| d.iter_mut().for_each(|u| *u = 0));
}

/** read all available frames from the websocket and relay each frame to the caller_id */
fn poll<E, R, S, T>(
    framer: &mut Framer<R, S>,
    stream: &mut T,
    frame_buf: &mut [u8],
    cid: CID,
    opcode: u32,
) where
    E: std::fmt::Debug,
    R: rand::RngCore,
    T: ws::framer::Stream<E>,
    S: ws::WebSocketType,
{
    while let Some(frame) = framer
        .read_binary(&mut *stream, frame_buf)
        .expect("failed to read websocket")
    {
        let frame: [u8; WEBSOCKET_BUFFER_LEN] = frame
            .try_into()
            .expect("websocket frame too large for buffer");
        let buf = Buffer::into_buf(Return::Frame(frame))
            .expect("failed to serialize websocket frame into buffer");
        buf.send(cid, opcode)
            .expect("failed to send websocket frame");
    }
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

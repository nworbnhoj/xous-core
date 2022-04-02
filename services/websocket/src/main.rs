#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
use api::*;
use embedded_websocket::{
    framer::Framer, WebSocketClient, WebSocketCloseStatusCode, WebSocketOptions,
    WebSocketSendMessageType,
};
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
use xous_ipc::Buffer;

struct WssStream<T: Read + Write>(T);

impl<T: Read + Write> embedded_websocket::framer::Stream<Error> for WssStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, Error> {
        self.0.read(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> std::result::Result<(), Error> {
        self.0.write_all(buf)
    }
}

struct Assets<R: rand::RngCore, T: Read + Write> {
    socket: WebSocketClient<R>,
    stream: WssStream<T>,
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

    // build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
    thread::spawn({
        let local_cid = xous::connect(ws_sid).unwrap();
        move || {
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            loop {
                tt.sleep_ms(KEEPALIVE_TIMEOUT_SECONDS.as_millis().try_into().unwrap())
                    .unwrap();
                xous::send_message(
                    local_cid,
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

    /*
    These buffers can be allocated here before the main loop, or alternatvely alocated and
    de-allocated within the loop as required ( presumably, trading memory with performance )
    TODO review/test/optimise buffer allocation
    */
    let mut read_buf = [0; WEBSOCKET_BUFFER_LEN];
    let mut read_cursor = 0;
    let mut write_buf = [0; WEBSOCKET_BUFFER_LEN];
    let mut _frame_buf = [0; WEBSOCKET_BUFFER_LEN];
    let mut _read_buf = [0; WEBSOCKET_BUFFER_LEN];

    /*
    store holds the assets of existing websockets by pid - and as such - limits each pid to 1 websocket.
    TODO review the limitation of 1 websocket per pid.
    */

    let mut store: HashMap<NonZeroU8, Assets<ThreadRng, StreamOwned<ClientConnection, TcpStream>>> =
        HashMap::new();

    log::trace!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(ws_sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Close) => {
                let pid = msg.sender.pid().unwrap();
                let (mut socket, mut stream) = match store.get_mut(&pid) {
                    Some(assets) => (&mut assets.socket, &mut assets.stream),
                    None => {
                        log::info!("Websocket assets not in list");
                        break;
                    }
                };
                let mut framer =
                    Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, &mut socket);

                match framer.close(stream, WebSocketCloseStatusCode::NormalClosure, None) {
                    Ok(()) => log::info!("Sent close handshake"),
                    Err(e) => log::info!("Failed to send close handshake {:?}", e),
                };
            }
            Some(Opcode::Open) => {
                let pid = msg.sender.pid().unwrap();
                if store.contains_key(&pid) {
                    log::info!("WebSocket already open for pid: {:?}", pid);
                    break;
                }
                let buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };
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

                // Create a TCP Stream between this client and the remote Server
                let tcp_stream = match TcpStream::connect(url.as_str()) {
                    Ok(tcp_stream) => tcp_stream,
                    Err(e) => {
                        log::info!("Failed to open TCP Stream {:?}", e);
                        break;
                    }
                };
                log::info!("TCP connected to {:?}", base_url);

                // Create a TLS connection to the remote Server on the TCP Stream
                // TODO pass ssl_config in during Opcode::Open ie ::from(ws_config.ssl_config);
                let tls_connector = RustlsConnector::from(local_ssl_config());
                let tls_stream = match tls_connector.connect(base_url, tcp_stream) {
                    Ok(stream) => {
                        log::info!("TLS connected to {:?}", base_url);
                        stream
                    }
                    Err(e) => {
                        log::info!("Failed to complete TLS handshake {:?}", e);
                        break;
                    }
                };

                // Initiate a websocket opening handshake over the TLS Stream
                let websocket_options = WebSocketOptions {
                    path: &path,
                    host: &base_url,
                    origin: "",
                    sub_protocols: None,
                    additional_headers: None,
                };
                let mut wss_stream = WssStream(tls_stream);

                let mut websocket_client = WebSocketClient::new_client(rand::thread_rng());

                let mut framer = Framer::new(
                    &mut read_buf,
                    &mut read_cursor,
                    &mut write_buf,
                    &mut websocket_client,
                );

                let sub_protocol = match framer.connect(&mut wss_stream, &websocket_options) {
                    Ok(sub_protocol) => sub_protocol,
                    Err(e) => {
                        log::info!("Unable to connect WebSocket {:?}", e);
                        break;
                    }
                };
                // TODO maybe subprotol should be saved in options??
                log::info!("WebSocket connected with: {:?}", sub_protocol);

                // Store the open websocket indexed by the calling pid
                store.insert(
                    pid,
                    Assets {
                        stream: wss_stream,
                        socket: websocket_client,
                    },
                );
            }
            Some(Opcode::Send) => xous::msg_scalar_unpack!(msg, msg_type, _, _, _, {
                let pid = msg.sender.pid().unwrap();
                let (mut socket, mut stream) = match store.get_mut(&pid) {
                    Some(assets) => (&mut assets.socket, &mut assets.stream),
                    None => {
                        log::info!("Websocket assets not in list");
                        break;
                    }
                };
                let ws_msg_type = match msg_type {
                    Text => WebSocketSendMessageType::Text,
                    Binary => WebSocketSendMessageType::Binary,
                    invalid => {
                        log::info!("Invalid value SendMessageType: {:?}", invalid);
                        break;
                    }
                };
                let frame_buf = unsafe {
                    Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                };

                let mut framer =
                    Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, &mut socket);

                match framer.write(stream, ws_msg_type, true, &frame_buf) {
                    Ok(()) => log::info!("Websocket frame sent"),
                    Err(e) => {
                        log::info!("failed to send Websocket frame {:?}", e);
                        break;
                    }
                };
            }),
            Some(Opcode::Tick) => {
                let pid = msg.sender.pid().unwrap();
                let (mut socket, mut stream) = match store.get_mut(&pid) {
                    Some(assets) => (&mut assets.socket, &mut assets.stream),
                    None => {
                        log::info!("Websocket assets not in list");
                        break;
                    }
                };
                let frame_buf = "keep alive please :-)".as_bytes();

                let mut framer =
                    Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, &mut socket);

                match framer.write(stream, WebSocketSendMessageType::Text, true, &frame_buf) {
                    Ok(()) => log::info!("Websocket keep-alive request sent"),
                    Err(e) => {
                        log::info!("failed to send Websocket keep-alive request {:?}", e);
                        break;
                    }
                };

                log::info!("Websocket keep-alive request sent");
            }

            Some(Opcode::Quit) => {
                log::warn!("got quit!");
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

// TODO pass ssl_config in during Opcode::Open ie ::from(ws_config.ssl_config);
// this is temporary Signal specific code below

use libsignal_service::configuration::{ServiceConfiguration, SignalServers};

fn local_ssl_config() -> rustls::ClientConfig {
    let cfg: ServiceConfiguration = SignalServers::Staging.into();
    //    let user_agent = "xous".to_string();
    let mut cert_bytes = std::io::Cursor::new(&cfg.certificate_authority);
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

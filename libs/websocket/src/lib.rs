#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
use api::{LendData, Opcode, Return, SendData, WsError, WsStream};

pub mod test;

use embedded_websocket as ws;
use num_traits::FromPrimitive;
use rand_chacha::rand_core::RngCore;
use rustls::{ServerName, StreamOwned};
use std::convert::{TryFrom, TryInto};
use std::io::{Error, ErrorKind};
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use url::Url;
use webpki_roots::TLS_SERVER_ROOTS;
use ws::framer::Framer;
use ws::WebSocketCloseStatusCode as StatusCode;
use ws::WebSocketSendMessageType as MessageType;
use ws::{WebSocketClient, WebSocketOptions, WebSocketState};
use xous::send_message;
use xous::{MemoryMessage, MemoryRange, MemorySize, Message, CID, SID};
use xous_ipc::Buffer;

/** time between reglar websocket keep-alive requests */
pub(crate) const KEEPALIVE_TIMEOUT_SECONDS: Duration = Duration::from_secs(55);
/** time between regular poll for inbound frames */
pub(crate) const LISTENER_POLL_INTERVAL_MS: Duration = Duration::from_millis(250);

pub(crate) const XOUS_MSG_PAGE_SIZE: usize = 4096;
/*
 A websocket header requires 2-14 bytes of the websocket buffer
 ( see https://crates.io/crates/embedded-websocket ) leaving the remainder
 available for the payload. This relates directly to the frame buffer.
*/
pub(crate) const WEBSOCKET_WRITE_LEN: usize = XOUS_MSG_PAGE_SIZE + 14;
pub(crate) const WEBSOCKET_READ_LEN: usize = XOUS_MSG_PAGE_SIZE;
/* frame_buf should be large enough to hold an entire websocket frame */
pub(crate) const WEBSOCKET_FRAME_LEN: usize = XOUS_MSG_PAGE_SIZE * 2;

pub struct Websocket<'a, TRng>
where
    TRng: RngCore,
{
    cid: CID,
    opcode: usize,
    url: Url,
    tls: bool,
    protocol: Option<&'a str>,
    /** the underlying tcp stream */
    tcp_stream: Option<TcpStream>,
    /** a websocket stream when opened on a tcp connection */
    ws_stream: Option<WsStream>,
    /** the framer read buffer */
    read_buf: [u8; WEBSOCKET_READ_LEN],
    /** the framer read cursor */
    cursor: usize,
    /** the framer write buffer */
    write_buf: [u8; WEBSOCKET_WRITE_LEN],

    ws_client: WebSocketClient<TRng>,
}

impl<'a, TRng> Websocket<'a, TRng>
where
    TRng: RngCore,
{
    /**
    tls:                true for a TLS connection - fallback to tcp
    base_url:           the url of the target websocket server
    path:               a path to apend to the url
    use_credentials:    true to authenticate
    login:              authentication username
    password:           authentication password
    cid:
    opcode:             the opcode for inbound data frames
    */
    pub fn new(
        host: &'a str,
        port: Option<u16>,
        path: Option<&'a str>,
        login: Option<&'a str>,
        password: Option<&'a str>,
        tls: bool,
        protocol: Option<&'a str>,
        cid: CID,
        opcode: usize,
        rng: TRng,
    ) -> Result<Websocket<'a, TRng>, Error> {
        // construct url
        let mut url = match Url::parse(host) {
            Ok(url) => url,
            Err(e) => {
                log::warn!("invalid websocket host {:?}", e);
                return Err(Error::from(ErrorKind::InvalidInput));
            }
        };
        if path.is_some() {
            url = url.join(path.unwrap()).expect("valid path");
        };
        if port.is_some() {
            url.set_port(port).expect("valid port");
        }
        if login.is_some() {
            url.query_pairs_mut().append_pair("login", login.unwrap());
        }
        if password.is_some() {
            url.query_pairs_mut()
                .append_pair("password", password.unwrap());
        }
        let scheme = match tls {
            true => "wss",
            false => "ws",
        };
        url.set_scheme(scheme).expect("fail set url scheme");

        Ok(Websocket {
            cid,
            opcode,
            url,
            tls,
            protocol,
            tcp_stream: None,
            ws_stream: None,
            read_buf: [0u8; WEBSOCKET_READ_LEN],
            cursor: 0,
            write_buf: [0u8; WEBSOCKET_WRITE_LEN],
            ws_client: WebSocketClient::new_client(rng),
        })
    }

    pub fn open(&mut self) -> Result<Option<String>, Error> {
        log::info!("Opening WebSocket");

        // Create a TCP Stream between this device and the remote Server
        let tcp_url = format!(
            "{}:{}",
            self.url.host_str().unwrap(),
            self.url.port().unwrap()
        );
        log::info!("Opening TCP connection to {:?}", tcp_url);
        let tcp_stream = match TcpStream::connect(tcp_url.clone()) {
            Ok(tcp_stream) => tcp_stream,
            Err(e) => {
                log::warn!("Failed to open TCP Stream {:?}", e);
                return Err(Error::from(ErrorKind::ConnectionRefused));
            }
        };
        log::info!("TCP connected to {:?}", tcp_url);

        self.tcp_stream = match tcp_stream.try_clone() {
            Ok(stream) => Some(stream),
            Err(e) => {
                log::error!("Failed to clone TCP Stream {}", e);
                None
            }
        };

        self.ws_stream = match self.tls {
            false => Some(WsStream::Tcp(tcp_stream)),
            true => {
                // Attempt to create a TLS connection to the remote Server on the TCP Stream
                let mut root_certs = rustls::RootCertStore::empty();
                root_certs.add_server_trust_anchors(TLS_SERVER_ROOTS.0.iter().map(|ta| {
                    rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
                        ta.subject,
                        ta.spki,
                        ta.name_constraints,
                    )
                }));

                let ssl_config = rustls::ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(root_certs)
                    .with_no_client_auth();
                let rc_config = Arc::new(ssl_config);

                let server_name = ServerName::try_from(self.url.host_str().unwrap())
                    .expect("Unable to construct ServerName from url");
                match rustls::ClientConnection::new(rc_config, server_name) {
                    Ok(client_connection) => {
                        log::info!(
                            "Established tls connection with {:?}",
                            self.url.host_str().unwrap()
                        );
                        Some(WsStream::Tls(StreamOwned::new(
                            client_connection,
                            tcp_stream,
                        )))
                    }
                    Err(e) => {
                        log::warn!("Failed to initiate tls client connection {}", e);
                        None // Abort connection if tls requested but not achieved??
                    }
                }
            }
        };

        // Initiate a websocket opening handshake over the WS Stream
        let mut framer = Framer::new(
            &mut self.read_buf,
            &mut self.cursor,
            &mut self.write_buf,
            &mut self.ws_client,
        );
        let sub_protocol;
        let mut sub_protocol_arr: [&str; 1] = [""];
        let sub_protocols: Option<&[&str]> = match self.protocol {
            Some(sp) => {
                sub_protocol = sp.to_string();
                sub_protocol_arr[0] = &sub_protocol;
                Some(&sub_protocol_arr[..])
            }
            None => None,
        };

        let options = WebSocketOptions {
            path: self.url.path(),
            host: self.url.host_str().unwrap(),
            origin: &self.url.origin().unicode_serialization(),
            sub_protocols,
            additional_headers: None,
        };
        let sub_protocol = match &mut self.ws_stream {
            Some(stream) => match framer.connect(stream, &options) {
                Ok(opt) => match opt {
                    Some(sp) => Some(sp.to_string()),
                    None => None,
                },
                Err(e) => {
                    log::warn!("Unable to connect WebSocket {:?}", e);
                    return Err(Error::from(ErrorKind::ConnectionRefused));
                }
            },
            None => {
                log::warn!("No stream available for Websocket");
                None
            }
        };

        log::info!("WebSocket connected with protocol: {:?}", sub_protocol);

        Ok(sub_protocol)
    }

    pub(crate) fn ok(&self) -> bool {
        self.ws_client.state == WebSocketState::Open
    }

    fn empty(&self) -> bool {
        match &self.tcp_stream {
            Some(stream) => {
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
            None => {
                log::warn!("called on non-open stream");
                true
            }
        }
    }

    /** read the frame from the websocket and relay to the caller_id */
    fn read(&mut self) -> Result<(), Error> {
        match &mut self.ws_stream {
            Some(stream) => {
                log::trace!("Reading Websocket Frame");
                let mut bytes = LendData([0u8; 4096]);
                let mut framer = Framer::new(
                    &mut self.read_buf,
                    &mut self.cursor,
                    &mut self.write_buf,
                    &mut self.ws_client,
                );
                match framer.read_binary(stream, &mut bytes.0[..]) {
                    Ok(Some(f)) => {
                        let len = f.len();
                        log::info!("Read {} bytes from Websocket", len);
                        let msg = MemoryMessage {
                            id: self.opcode,
                            buf: unsafe {
                                MemoryRange::new(
                                    &mut bytes as *mut LendData as usize,
                                    core::mem::size_of::<SendData>(),
                                )
                                .unwrap()
                            },
                            offset: None,
                            valid: MemorySize::new(len),
                        };
                        log::info!("Send bytes to cid={} (relay) {:?}", self.cid, msg);
                        match send_message(self.cid, Message::Borrow(msg)) {
                            Ok(_) => {
                                log::info!("Websocket Relayed {} bytes to cid={}", len, self.cid)
                            }
                            Err(e) => log::error!("failed to relay websocket frame: {:?}", e),
                        }
                        Ok(())
                    }
                    Ok(None) => Ok(()),
                    Err(_) => todo!(),
                }
            }
            None => {
                log::warn!("called on non-open stream");
                Ok(())
            }
        }
    }

    fn write(&mut self, buffer: &[u8]) -> Result<(), Error> {
        log::info!("Writing Websocket Frame");
        let mut ret = Ok(());

        match &mut self.ws_stream {
            Some(stream) => {
                let mut framer = Framer::new(
                    &mut self.read_buf,
                    &mut self.cursor,
                    &mut self.write_buf,
                    &mut self.ws_client,
                );
                let mut end_of_message = false;
                let mut start = 0;
                let mut slice;
                while !end_of_message {
                    log::trace!("start = {:?}", start);
                    if buffer.len() <= (start + XOUS_MSG_PAGE_SIZE) {
                        end_of_message = true;
                        slice = &buffer[start..];
                    } else {
                        slice = &buffer[start..(start + XOUS_MSG_PAGE_SIZE)];
                    }
                    ret = match framer.write(stream, MessageType::Binary, end_of_message, slice) {
                        Ok(ret) => Ok(ret),
                        Err(_e) => {
                            return Err(Error::new(
                                ErrorKind::BrokenPipe,
                                "Failed to write websocket frame",
                            ))
                        }
                    };
                    start = start + XOUS_MSG_PAGE_SIZE;
                }
                ret
            }
            None => {
                log::warn!("called on non-open stream");
                Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "called on non-open stream",
                ))
            }
        }
    }

    fn close(&mut self) -> Result<(), Error> {
        match &mut self.ws_stream {
            Some(stream) => {
                let mut framer = Framer::new(
                    &mut self.read_buf,
                    &mut self.cursor,
                    &mut self.write_buf,
                    &mut self.ws_client,
                );
                match framer.close(stream, StatusCode::NormalClosure, None) {
                    Ok(_ret) => Ok(()),
                    Err(e) => {
                        log::warn!("Failed to close WebSocket {:?}", e);
                        return Err(Error::from(ErrorKind::Other));
                    }
                }
            }
            None => {
                log::warn!("called on non-open stream");
                Ok(())
            }
        }
    }
}

pub fn server<TRng>(sid: SID, websocket: &mut Websocket<TRng>) -> !
where
    TRng: RngCore,
{
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let cid = xous::connect(sid).unwrap();

    match websocket.open() {
        Err(e) => {
            log::warn!("Unable to open Websocket - sorry it didnt work out. {}", e);
        }
        Ok(_protocol) => {
            // build a thread that emits a regular Opcode::Tick to send a KeepAliveRequest
            spawn_tick_pump(cid);

            // build a thread that emits a regular Opcode::Poll to receive inbound frames
            spawn_poll_pump(cid);

            log::info!("tick & poll pumps started");

            log::info!("ready to accept requests");
            loop {
                let mut msg = xous::receive_message(sid).unwrap();
                match FromPrimitive::from_usize(msg.body.id()) {
                    Some(Opcode::Close) => {
                        log::info!("Websocket Opcode::Close");
                        if !validate_msg(&mut msg, WsError::Scalar, Opcode::Close.into()) {
                            continue;
                        }
                        match websocket.close() {
                            Ok(_) => log::info!("Sent close handshake"),
                            Err(_e) => {
                                log::info!("Failed to send websocket close handshake")
                            }
                        }
                        log::info!("Websocket Opcode::Close complete");
                    }
                    Some(Opcode::Poll) => {
                        log::trace!("Websocket Poll Start");
                        if !websocket.empty() {
                            match websocket.read() {
                                Ok(_) => log::trace!("Websocket Read OK"),
                                Err(e) => log::info!("Websocket Read Error {:?}", e),
                            }
                        }
                        log::trace!("Websocket Poll Complete");
                    }
                    Some(Opcode::Send) => {
                        log::info!("Websocket Opcode::Send");
                        if !validate_msg(&mut msg, WsError::Memory, Opcode::Send.into()) {
                            continue;
                        }
                        let meta = msg.body.to_usize();
                        let mut buf = unsafe {
                            Buffer::from_memory_message(msg.body.memory_message().unwrap())
                        };
                        let len = meta[5];
                        match websocket.write(&buf[..len]) {
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
                        if !validate_msg(&mut msg, WsError::ScalarBlock, Opcode::State.into()) {
                            continue;
                        }
                        let state = match websocket.ok() {
                            true => 1,
                            false => 0,
                        };
                        xous::return_scalar(msg.sender, state)
                            .expect("failed to return WebSocketState");
                        log::info!("Websocket Opcode::State complete");
                    }
                    Some(Opcode::Tick) => {
                        log::info!("Websocket Opcode::Tick");
                        if !validate_msg(&mut msg, WsError::Scalar, Opcode::Tick.into()) {
                            continue;
                        }
                        // TODO review keep alive request technique
                        let buf = "keep alive please :-)".as_bytes();
                        match websocket.write(&buf[..]) {
                            Ok(()) => log::info!("Websocket tick sent"),
                            Err(e) => {
                                log::warn!("failed to send Websocket tick: {:?}", e);
                                // let hint = format!("failed to send Websocket tick {:?}", e);
                                // buf.replace(drop(&hint)).expect("failed replace buffer");
                                continue;
                            }
                        };
                        log::info!("Websocket Opcode::Tick complete");
                    }

                    Some(Opcode::Quit) => {
                        log::warn!("Websocket Opcode::Quit");
                        if !validate_msg(&mut msg, WsError::Scalar, Opcode::Quit.into()) {
                            continue;
                        }
                        break;
                    }
                    None => {
                        log::error!("couldn't convert opcode: {:?}", msg);
                    }
                }
            }
            log::trace!("main loop exit, destroying servers");
        }
    }
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
                        Opcode::Tick as usize,
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

// build a thread that emits a regular WebSocketOp::Poll to send a PollRequest
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
                        Opcode::Poll as usize,
                        LISTENER_POLL_INTERVAL_MS.as_millis().try_into().unwrap(),
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

pub(crate) fn validate_msg(
    env: &mut xous::MessageEnvelope,
    expected: WsError,
    opcode: usize,
) -> bool {
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

/** helper function to return hints from opcode panics */
pub(crate) fn drop(hint: &str) -> Return {
    log::warn!("{}", hint);
    Return::Failure(xous_ipc::String::from_str(hint))
}

mod ws_test_server;

use num_traits::{FromPrimitive, ToPrimitive};
use std::thread;
use websocket::{Opcode, Return, WebsocketConfig, SERVER_NAME_WEBSOCKET};
use xous::send_message;
use xous::Message;
use xous_ipc::Buffer;

const WS_TEST_NAME: &str = "_ws_test_";
const TEST_MSG_SIZE: usize = 32;
const PROTOCOL: &str = "echo";

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
enum TestOpcode {
    Receive,
}

pub fn local(tls: bool) {
    //log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());
    
    log::info!("Starting local websocket server");
    thread::spawn({
        move || {
            ws_test_server::main().unwrap();
        }
    });
    log::info!("Started local websocket server on 127.0.0.1:1337");

    let certificate_authority = match tls {
        true => Some(xous_ipc::String::from_str(
            "TODO invalid certificate authority",
        )),
        false => None,
    };

    let _tt = ticktimer_server::Ticktimer::new().unwrap();
    let xns = xous_names::XousNames::new().unwrap();
    let sid = xous::create_server().expect("couldn't create ws_test server");
    let cid: u32 = xous::connect(sid).unwrap();

    // prepare to open websocket
    let ws_cid = xns
        .request_connection_blocking(SERVER_NAME_WEBSOCKET)
        .expect("Cannot connect to websocket server");
    let config = WebsocketConfig {
        cid: cid,
        opcode: TestOpcode::Receive.to_u32().unwrap(),
        host: xous_ipc::String::from_str("ws://127.0.0.1"),
        port: Some(xous_ipc::String::from_str("1337")),
        path: Some(xous_ipc::String::from_str("/test")),
        login: None,
        password: None,
        certificate_authority: certificate_authority,
        sub_protocol: Some(xous_ipc::String::from_str(PROTOCOL)),
    };
    log::trace!("Opening websocket with {:#?}", config);

    // Request the websocket_client_service to open a websocket with WebsocketConfig
    let mut buf = Buffer::into_buf(config)
        .or(Err(xous::Error::InternalError))
        .expect("failed to construct WebsocketConfig buffer");
    buf.lend_mut(ws_cid, Opcode::Open.to_u32().unwrap())
        .map(|_| ())
        .expect("request to open websocket failed");

    match buf.to_original::<Return, _>().unwrap() {
        Return::SubProtocol(protocol) => match protocol {
            Some(text) => {
                match text.to_str() {
                    "echo" => log::info!("Opened WebSocket with protocol echo"),
                    _ => log::info!("FAIL: protocol != echo : {:?}", text.to_str()),
                };
            }
            None => log::info!(
                "FAIL: protocol missing https://github.com/ninjasource/embedded-websocket/pull/10)"
            ),
        },
        Return::Failure(hint) => log::info!("FAIL: on retrieve protocol: {:?}", hint),
    };

    // check that Websocket is open
    match send_message(
        ws_cid,
        Message::new_blocking_scalar(Opcode::State.to_usize().unwrap(), 0, 0, 0, 0),
    ) {
        Ok(xous::Result::Scalar1(state)) => match state {
            0 => log::info!("FAIL Websocket state is not open"),
            1 => log::info!("Websocket state is open"),
            _ => log::info!("FAIL Websocket state unknown"),
        },
        _ => log::info!("FAIL Unable to retrieve Websocket state"),
    }

    // Send test bytes via websocket

    let outbound: xous_ipc::String<TEST_MSG_SIZE> = xous_ipc::String::from_str("please echo me");
    let buf = Buffer::into_buf(outbound).expect("failed put msg in buffer");

    log::info!("Outbound Buffer {:?}", &buf[..64]);

    buf.send(ws_cid, Opcode::Send.to_u32().unwrap())
        .map(|_| ())
        .expect("failed to send via websocket");

    // wait around for test bytes from the websocket
    let mut msg = xous::receive_message(sid).unwrap();
    match FromPrimitive::from_usize(msg.body.id()) {
        Some(TestOpcode::Receive) => {
            log::info!("Received TestOpcode::Receive");
            let buf = unsafe {
                Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
            };
            let inbound = buf
                .to_original::<xous_ipc::String<TEST_MSG_SIZE>, _>()
                .unwrap();
            log::info!("Completed TestOpcode::Receive: {}", inbound);
        }
        None => {
            log::error!("couldn't convert opcode: {:?}", msg);
        }
    }

    // close the websocket
    log::info!("Closing websocket");
    xous::send_message(
        ws_cid,
        xous::Message::new_scalar(Opcode::Close.to_usize().unwrap(), 0, 0, 0, 0),
    )
    .expect("couldn't send test_app quit");
    log::info!("Closed websocket OK");
}

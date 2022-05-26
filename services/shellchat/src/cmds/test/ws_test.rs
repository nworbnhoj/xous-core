use num_traits::{ToPrimitive};
use websocket::{Opcode, WebsocketConfig, SERVER_NAME_WEBSOCKET};
use xous_ipc::Buffer;

const PROTOCOL: &str = "echo";

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
enum TestOpcode {
    Receive,
}

pub fn local() {
    //log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());

    let _tt = ticktimer_server::Ticktimer::new().unwrap();
    let xns = xous_names::XousNames::new().unwrap();
    let _sid = xous::create_server().expect("couldn't create ws_test server");

    // prepare to open websocket
    let ws_cid = xns
        .request_connection_blocking(SERVER_NAME_WEBSOCKET)
        .expect("Cannot connect to websocket server");
    let config = WebsocketConfig {
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

}

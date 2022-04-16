mod ws_test_server;

#[cfg(feature = "ws_test")]
pub mod local {

    use crate::api::*;
    use crate::WebsocketConfig;
    use num_traits::{FromPrimitive, ToPrimitive};
    use std::io::{Error, ErrorKind};
    use std::thread;
    use xous::CID;
    use xous_ipc::Buffer;

    const WS_TEST_NAME: &str = "_ws_test_";
    const TEST_MSG_SIZE: usize = 128;

    #[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
    enum TestOpcode {
        Send,
        Receive,
        Quit,
    }

    pub fn main(tls: bool) {
        log::info!("Start local websocket server");
        thread::spawn({
            move || {
                super::ws_test_server::main();
            }
        });
        log::info!("Started local websocket server on 127.0.0.1:1337");

        let ca = match tls {
            true => Some(xous_ipc::String::from_str(
                "TODO invalid certificate authority",
            )),
            false => None,
        };

        // start test_app
        thread::spawn({
            move || {
                test_app(ca);
            }
        });

        let tt = ticktimer_server::Ticktimer::new().unwrap();
        let xns = xous_names::XousNames::new().unwrap();

        // pause to allow test_app to get up and running
        tt.sleep_ms(1000).expect("insomnia");
        let test_app_cid = xns
            .request_connection_blocking(WS_TEST_NAME)
            .expect("Cannot connect to test_app");

        // send a test message via websocket
        xous::send_message(
            test_app_cid,
            xous::Message::new_scalar(TestOpcode::Send.to_usize().unwrap(), 0, 0, 0, 0),
        )
        .expect("failed to send test_app msg");

        // pause to allow local websocket server to echo messages
        tt.sleep_ms(1000).expect("insomnia");

        // quit the test_app (triggers websocket close)
        xous::send_message(
            test_app_cid,
            xous::Message::new_scalar(TestOpcode::Quit.to_usize().unwrap(), 0, 0, 0, 0),
        )
        .expect("failed to send test_app quit");
    }

    fn test_app(ca: Option<xous_ipc::String<CA_LEN>>) {
        // register this test_app with xous
        let xns = xous_names::XousNames::new().unwrap();
        let ws_test_sid = xns
            .register_name(WS_TEST_NAME, None)
            .expect("can't register server");
        log::trace!("registered with NS -- {:?}", ws_test_sid);

        log::info!("open websocket");
        let ws_cid = match open_websocket(ca) {
            Ok(cid) => cid,
            Err(e) => {
                log::info!("Failed to open websocket over tls {:?}", e);
                return;
            }
        };
        log::info!(" websocket open OK");

        loop {
            let mut msg = xous::receive_message(ws_test_sid).unwrap();
            match FromPrimitive::from_usize(msg.body.id()) {
                Some(TestOpcode::Send) => {
                    let outbound: xous_ipc::String<TEST_MSG_SIZE> = xous_ipc::String::from_str(
                        "send this message outbound from test_app via websocket",
                    );
                    let mut buf =
                        Buffer::into_buf(outbound).expect("failed put msg in buffer");
                    buf.lend_mut(ws_cid, Opcode::Send.to_u32().unwrap())
                        .map(|_| ())
                        .expect("failed to send via websocket");
                }
                Some(TestOpcode::Receive) => {
                    let buf = unsafe {
                        Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                    };
                    let inbound = buf
                        .to_original::<xous_ipc::String<TEST_MSG_SIZE>, _>()
                        .unwrap();
                    log::info!("received inbound via websocket: {}", inbound);
                }
                Some(TestOpcode::Quit) => break,
                None => {
                    log::error!("couldn't convert opcode: {:?}", msg);
                }
            }
        }

        log::info!("close websocket");
        xous::send_message(
            ws_cid,
            xous::Message::new_scalar(Opcode::Close.to_usize().unwrap(), 0, 0, 0, 0),
        )
        .expect("couldn't send test_app quit");
        log::info!("close websocket OK");
    }

    fn open_websocket(
        certificate_authority: Option<xous_ipc::String<CA_LEN>>,
    ) -> Result<CID, Error> {
        let xns = xous_names::XousNames::new().unwrap();
        let sid = xns
            .register_name("_test websocket_", None)
            .expect("can't register server");
        let cid: u32 = xous::connect(sid).unwrap();

        let ws_cid = xns
            .request_connection_blocking(SERVER_NAME_WEBSOCKET)
            .expect("Cannot connect to websocket server");

        let config = WebsocketConfig {
            certificate_authority: certificate_authority,
            base_url: xous_ipc::String::from_str("http://127.0.0.1:1337"),
            path: xous_ipc::String::from_str(""),
            use_credentials: false,
            login: xous_ipc::String::from_str(""),
            password: xous_ipc::String::from_str(""),
            cid: cid,
            opcode: TestOpcode::Receive.to_u32().unwrap(),
        }; 
        log::info!("opening websocket with {:?}", config);        
        
        // Request the websocket_client_service to open a websocket with WebsocketConfig
        let mut buf = Buffer::into_buf(config)
            .or(Err(xous::Error::InternalError))
            .expect("failed to construct WebsocketConfig buffer");
        buf.lend_mut(ws_cid, Opcode::Open.to_u32().unwrap())
            .map(|_| ())
            .expect("request to open websocket failed");

        match buf.to_original::<Return, _>().unwrap() {
            Return::Frame(_frame) => Err(Error::new(ErrorKind::Other, "invalid Return type")),
            Return::SubProtocol(protocol) => {
                assert_eq!(protocol, protocol); // TODO test sub_protocol
                Ok(ws_cid)
            }
            Return::Failure(_hint) => Err(Error::new(ErrorKind::Other, "invalid Return type")),
        }
    }
}

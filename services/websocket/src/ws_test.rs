mod ws_test_server;

#[cfg(feature = "ws_test")]
pub mod local {

    use crate::api::*;
    use crate::WebsocketConfig;
    use num_traits::{FromPrimitive, ToPrimitive};
    use std::any::Any;
    use std::io::{Error, ErrorKind};
    use std::thread;
    use xous::CID;
    use xous_ipc::Buffer;

    #[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
    enum TestOpcode {
        /// receive a websocket frame
        Frame,
    }

    pub fn tcp() {
        match start_websocket_server_local() {
            Ok(()) => log::info!("Started local websocket server on 127.0.0.1:1337"),
            Err(e) => {
                log::info!("Failed to start local websocket server {:?}", e);
                return;
            }
        }
        let ws_cid = match open_websocket(None) {
            Ok(cid) => cid,
            Err(e) => {
                log::info!("Failed to open websocket over tcp {:?}", e);
                return;
            }
        };
        log::info!("Opened websocket over tcp");
    }

    pub fn tls() {
        match start_websocket_server_local() {
            Ok(()) => log::info!("Started local websocket server on 127.0.0.1:1337"),
            Err(e) => {
                log::info!("Failed to start local websocket server {:?}", e);
                return;
            }
        }
        let ca = xous_ipc::String::from_str("TODO invalid certificate authority");
        let ws_cid = match open_websocket(Some(ca)) {
            Ok(cid) => cid,
            Err(e) => {
                log::info!("Failed to open websocket over tls {:?}", e);
                return;
            }
        };
        log::info!("Opened websocket over tls");
    }

    // start local websocket test server at 127.0.0.1:1337
    fn start_websocket_server_local() -> Result<(), Box<(dyn Any + Send + 'static)>> {
        thread::spawn({
            move || {
                super::ws_test_server::main().expect("failed to start websocket test server");
            }
        })
        .join()
    }

    fn open_websocket(
        certificate_authority: Option<xous_ipc::String<CA_LEN>>,
    ) -> Result<CID, Error> {
        log_server::init_wait().unwrap();
        log::set_max_level(log::LevelFilter::Info);
        log::info!("my PID is {}", xous::process::id());

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
            base_url: xous_ipc::String::from_str("127.0.0.1:1337"),
            path: xous_ipc::String::from_str(""),
            use_credentials: false,
            login: xous_ipc::String::from_str(""),
            password: xous_ipc::String::from_str(""),
            cid: cid,
            opcode: TestOpcode::Frame.to_u32().unwrap(),
        };

        // Request the websocket_client_service to open a websocket with WebsocketConfig
        let buf = Buffer::into_buf(config)
            .or(Err(xous::Error::InternalError))
            .expect("failed to construct WebsocketConfig buffer");
        buf.lend(ws_cid, Opcode::Open.to_u32().unwrap())
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

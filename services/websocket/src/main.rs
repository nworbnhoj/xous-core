#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
use api::*;

use std::{sync::Arc, time::Duration};
use num_traits::FromPrimitive;
use std::convert::TryInto;
use std::thread;

use embedded_websocket::*;
use libsignal_services::*;




#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let ws_sid = xns.register_name(api::SERVER_NAME_WEBSOCKET, None).expect("can't register server");
    log::trace!("registered with NS -- {:?}", ws_sid);
    
    // build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
    thread::spawn({
        let local_cid = xous::connect(ws_sid).unwrap();
        move || {
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            loop {
                tt.sleep_ms(KEEPALIVE_TIMEOUT_SECONDS.as_millis).unwrap();
                xous::send_message(local_cid,
                    xous::Message::new_scalar(Opcode::Tick.to_usize().unwrap(),
                    KEEPALIVE_TIMEOUT_SECONDS, 0, 0, 0)
                ).expect("couldn't send Websocket tick");
            }
        }
    });


    log::trace!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(ws_sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Tick) => {
                if let Err(e) =
                    incoming_sink.send(WebSocketStreamItem::KeepAliveRequest)
                {
                    log::info!("Websocket sink has closed: {:?}.", e);
                    break;
                }
                log::info!("Websocket keep-alive request sent");
            },
            Some(Opcode::Quit) => {
                log::warn!("got quit!");
                break
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

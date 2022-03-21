#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
use api::*;

use num_traits::FromPrimitive;


#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let ws_sid = xns.register_name(api::SERVER_NAME_WEBSOCKET, None).expect("can't register server");
    log::trace!("registered with NS -- {:?}", ws_sid);


    log::trace!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(ws_sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
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

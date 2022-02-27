#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
use api::*;

use num_traits::FromPrimitive;

use log::info;

#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    info!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let espeak_sid = xns.register_name(api::SERVER_NAME_ESPEAK).expect("can't register server");
    log::trace!("registered with NS -- {:?}", espeak_sid);

    loop {
        let msg = xous::receive_message(espeak_sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::StrToWav) => xous::msg_scalar_unpack!(msg, token, _, _, _, {
            }),
            None => {
                log::error!("couldn't convert opcode");
                break
            }
        }
    }
    // clean up our program
    log::trace!("main loop exit, destroying servers");
    xns.unregister_server(espeak_sid).unwrap();
    xous::destroy_server(espeak_sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}

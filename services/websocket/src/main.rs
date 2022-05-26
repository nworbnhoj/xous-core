#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
mod manager;

use api::{Opcode, Return, WebsocketConfig, WsError};
use manager::Opcode as ClientOp;
use num_traits::{FromPrimitive, ToPrimitive};
use std::thread;

#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let ws_sid = xns
        .register_name(api::SERVER_NAME_WEBSOCKET, None)
        .expect("can't register server");
    log::trace!("registered with NS -- {:?}", ws_sid);
    let _ws_cid = xous::connect(ws_sid).unwrap();

    // get the websocket Manager up and running
    let ws_manager_sid = xous::create_server().expect("couldn't create websocket client server");
    let ws_manager_cid = xous::connect(ws_manager_sid).unwrap();
    thread::spawn({
        move || {
            manager::main(ws_manager_sid);
        }
    });

    log::trace!("ready to accept requests");
    loop {
        let mut msg = xous::receive_message(ws_sid).unwrap();
        let _response = match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Close) => {
                log::info!("Websocket Opcode::Close");
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Close.to_u32().unwrap()) {
                    continue;
                }
                let rsp = msg
                    .forward(ws_manager_cid, ClientOp::Close as _)
                    .expect("failed to forward Opcode::Close");
                log::info!("Websocket Opcode::Close complete");
                rsp
            }
            Some(Opcode::Open) => {
                log::info!("Websocket Opcode::Open");
                if !validate_msg(
                    &mut msg,
                    WsError::MemoryBlock,
                    Opcode::Open.to_u32().unwrap(),
                ) {
                    continue;
                }
                let rsp = msg
                    .forward(ws_manager_cid, ClientOp::Open as _)
                    .expect("failed to forward Opcode::Open");
                log::info!("Websocket Opcode::Open complete");
                rsp
            }
            Some(Opcode::Send) => {
                if !validate_msg(&mut msg, WsError::Memory, Opcode::Send.to_u32().unwrap()) {
                    continue;
                }
                log::info!("Websocket Opcode::Send");
                let rsp = msg
                    .forward(ws_manager_cid, ClientOp::Send as _)
                    .expect("failed to forward Opcode::Send");

                log::info!("Websocket Opcode::Send complete");
                rsp
            }
            Some(Opcode::State) => {
                log::info!("Websocket Opcode::State");
                if !validate_msg(
                    &mut msg,
                    WsError::ScalarBlock,
                    Opcode::State.to_u32().unwrap(),
                ) {
                    continue;
                }
                let rsp = msg
                    .forward(ws_manager_cid, ClientOp::State as _)
                    .expect("failed to forward Opcode::State");
                log::info!("Websocket Opcode::State complete");
                rsp
            }

            Some(Opcode::Quit) => {
                log::warn!("Websocket Opcode::Quit");
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Quit.to_u32().unwrap()) {
                    continue;
                }
                msg.forward(ws_manager_cid, ClientOp::Quit as _)
                    .expect("failed to forward Opcode::Quit");
                log::info!("Websocket Opcode::State complete");
                break;
            }
            None => {
                log::error!("couldn't convert opcode: {:?}", msg);
            }
        };
    }
    // clean up our program
    log::trace!("main loop exit, destroying servers");
    xns.unregister_server(ws_sid).unwrap();
    xous::destroy_server(ws_sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}

pub(crate) fn validate_msg(
    env: &mut xous::MessageEnvelope,
    expected: WsError,
    opcode: u32,
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

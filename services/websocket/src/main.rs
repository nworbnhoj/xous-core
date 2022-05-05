#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
mod manager;

use api::{validate_msg, Opcode, Return, WebsocketConfig, WsError};

use manager::Opcode as ClientOp;
use num_traits::{FromPrimitive, ToPrimitive};

use xous_ipc::Buffer;

use std::thread;

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
    let ws_cid = xous::connect(ws_sid).unwrap();

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
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Close) => {
                log::info!("Websocket Opcode::Close");
                if !validate_msg(&mut msg, WsError::Scalar, Opcode::Close.to_u32().unwrap()) {
                    continue;
                }
                let response = msg
                    .forward(ws_manager_cid, ClientOp::Close as _)
                    .expect("failed to forward Opcode::Close");
                log::info!("Websocket Opcode::Close complete");
                response
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
                let pid = msg.sender.pid().unwrap();
                let response = msg
                    .forward(ws_manager_cid, ClientOp::Open as _)
                    .expect("failed to forward Opcode::Open");
                log::info!("Websocket Opcode::Open complete");
            }
            Some(Opcode::Send) => {
                if !validate_msg(&mut msg, WsError::Memory, Opcode::Send.to_u32().unwrap()) {
                    continue;
                }
                log::info!("Websocket Opcode::Send");
                let response = msg
                    .forward(ws_manager_cid, ClientOp::Send as _)
                    .expect("failed to forward Opcode::Send");

                log::info!("Websocket Opcode::Send complete");
                response
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
                let response = msg
                    .forward(ws_manager_cid, ClientOp::State as _)
                    .expect("failed to forward Opcode::State");
                log::info!("Websocket Opcode::State complete");
                response
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
        }
    }
    // clean up our program
    log::trace!("main loop exit, destroying servers");
    xns.unregister_server(ws_sid).unwrap();
    xous::destroy_server(ws_sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}

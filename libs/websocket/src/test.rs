mod test_server;

use super::{LendData, Opcode, Websocket, Error, server};
use num_traits::{FromPrimitive, ToPrimitive};
use rand::rngs::OsRng;
use std::thread;
use xous::send_message;
use xous::{MemoryMessage, MemoryRange, MemorySize, Message, CID};
use xous_ipc::Buffer;

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
enum TestOpcode {
    Receive,
}

pub fn local(tls: bool) -> Result<bool, Error> {
    //log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());
    let sid = xous::create_server().unwrap();
    let cid = xous::connect(sid).expect("failed get CID for ws_test");

    log::info!("Starting local websocket server");
    thread::spawn({
        move || {
            test_server::main().unwrap();
        }
    });
    log::info!("Started local websocket server on 127.0.0.1:1337");

    let mut websocket = Websocket::new(
        "ws://127.0.0.1",
        Some(1337),
        Some("/test"),
        None,
        None,
        tls,
        Some("test"),
        cid,
        0,
        OsRng,
    )
    .expect("failed to create Websocket struct");
    log::info!("Starting local websocket server");

    let ws_sid = xous::create_server().unwrap();
    let ws_cid: CID = xous::connect(ws_sid).expect("failed get CID for ws_test");

    thread::spawn({
        move || {
            server(ws_sid, &mut websocket);
        }
    });

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
    let test_msg = String::from("please echo me");
    let mut bytes = LendData([0u8; 4096]);
    for (dest, src) in bytes.0.iter_mut().zip(test_msg.bytes()) {
        *dest = src;
    }
    let msg = MemoryMessage {
        id: Opcode::Send as usize, // .into(),
        buf: unsafe {
            MemoryRange::new(
                &mut bytes as *mut LendData as usize,
                core::mem::size_of::<LendData>(),
            )
            .unwrap()
        },
        //buf: unsafe { MemoryRange::new(bytes.0.as_ptr() as _, 4096).unwrap() },
        offset: None,
        valid: MemorySize::new(test_msg.len()),
    };
    log::info!("Send test message {:?}", msg);
    send_message(ws_cid, Message::Borrow(msg)).expect("failed to send via websocket");

    log::info!("Waiting for echo back from test websocket server");

    let msg = xous::receive_message(sid).unwrap();
    log::info!("Received message {:?}", msg);
    let mut success = false;
    match FromPrimitive::from_usize(msg.body.id()) {
        Some(TestOpcode::Receive) => {
            log::info!("TestOpcode::Receive");
            let len = msg.body.to_usize()[5];
            let buf = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
            let echo = std::str::from_utf8(&buf[..len]).unwrap();
            log::info!("*** {} ***", &echo);
            success = String::from(echo) == test_msg;
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
    Ok(success)
}

# Websocket

This basic Xous websocket server is based on [embedded-websocket](https://crates.io/crates/embedded-websocket) [rustls](https://crates.io/crates/rustls) and [webpki-roots](https://crates.io/crates/webpki-roots)

Once a Websocket instance is created and server started then a websocket is opened and maintained until and `Opcode::Close` is received. Websocket frames can be sent with `websocket::send()` and inbound websocket frames are forwarded by the server to the `cid` and `opcode` provided.

## Example

```Rust
// configure websocket
let mut websocket = Websocket::new(
    "signal.org",         // host
    Some(1337),           // port
    Some("/test"),        // path
    None,                 // username
    None,                 // password
    false,                // tls connection
    Some("test"),         // sub-protocol request
    cid,                  // cid callback for inbound ws frames
    0,                    // opcode callback for inboud ws frames
    OsRng,                // random number gen
)

// start websocket server
let ws_sid = xous::create_server().unwrap();
let ws_cid: CID = xous::connect(ws_sid).expect("failed");
thread::spawn({
    move || {
        crate::websocket::server(ws_sid, &mut websocket);
    }
});

// send bytes via websocket
let outbound = String::from("test message");
let mut bytes = LendData([0u8; 4096]);
for (dest, src) in bytes.0.iter_mut().zip(outbound.bytes()) {
    *dest = src;
}
let msg = MemoryMessage {
    id: Opcode::Send as usize,
    buf: unsafe { MemoryRange::new(bytes.0.as_ptr() as _, 4096).unwrap() },
    offset: None,
    valid: MemorySize::new(test_msg.len()),
};
send_message(ws_cid, Message::Borrow(msg)).expect("failed");

// receive bytes via websocket
let msg = xous::receive_message(sid).unwrap();
let len = msg.body.to_usize()[5];
let buf = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
let inbound = std::str::from_utf8(&buf[..len]).unwrap().to_string();

// close websocket server
send_message(ws_cid, Message::new_blocking_scalar(Opcode::Close.into(), 0, 0, 0, 0));
```

## Notes

* [embedded-websocket](https://crates.io/crates/embedded-websocket) does not support the negotiation of sub-protocols (see [pull request](https://github.com/ninjasource/embedded-websocket/pull/10)).

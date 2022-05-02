#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

use super::*;
use crate::manager::Frame;
use api::*;

use embedded_websocket as ws;
use rand::rngs::OsRng;

use std::{convert::TryInto, io::ErrorKind, net::TcpStream};
use ws::framer::Framer;
use ws::{WebSocketClient, WebSocketState};
use xous::CID;
use xous_ipc::Buffer;

use std::time::Duration;

/** time between regular poll for inbound frames */
pub(crate) const LISTENER_POLL_INTERVAL_MS: Duration = Duration::from_millis(250);

/*
 A websocket header requires at least 14 bytes of the websocket buffer
 ( see https://crates.io/crates/embedded-websocket ) leaving the remainder
 available for the payload. This relates directly to the frame buffer.
 There may be advantage in independently specifying the read, frame, and write buffer sizes.
 TODO review/test/optimise WEBSOCKET_BUFFER_LEN
*/
pub(crate) const WEBSOCKET_BUFFER_LEN: usize = 4096;
pub(crate) const WEBSOCKET_PAYLOAD_LEN: usize = 4080;

pub(crate) struct Poll<'a> {
    /** the configuration of an open websocket */
    socket: WebSocketClient<OsRng>,
    /** a websocket stream */
    ws_stream: WsStream,
    /** the underlying tcp stream */
    tcp_stream: TcpStream,
    /** the callback_id to use when relaying an inbound websocket frame */
    cid: CID,
    /** the opcode to use when relaying an inbound websocket frame */
    opcode: u32,
    /** **/
    framer: Framer<'a, rand::rngs::OsRng, embedded_websocket::Client>,
}

impl<'a> Poll<'a> {
    pub(crate) fn new(
        cid: CID,
        opcode: u32,
        tcp_stream: TcpStream,
        ws_stream: WsStream,
        socket: WebSocketClient<OsRng>,
    ) -> Self {
        let mut read_buf = [0; WEBSOCKET_BUFFER_LEN];
        let mut read_cursor = 0;
        let mut write_buf = [0; WEBSOCKET_BUFFER_LEN];

        Poll {
            socket: socket,
            ws_stream: ws_stream,
            tcp_stream: tcp_stream,
            cid: cid,
            opcode: opcode,
            framer: Framer::new(&mut read_buf, &mut read_cursor, &mut write_buf, &mut socket),
        }
    }

    pub fn main(&self) {
        let tt = ticktimer_server::Ticktimer::new().unwrap();

        loop {
            tt.sleep_ms(LISTENER_POLL_INTERVAL_MS.as_millis().try_into().unwrap())
                .unwrap();

            log::trace!("Websocket Poll");

            if self.framer.state() != WebSocketState::Open {
                break;
            }

            if self.empty() {
                continue;
            }

            log::trace!("Websocket Read");
            self.read();
            log::trace!("Websocket Read complete");
        }
    }

    fn empty(&self) -> bool {
        self.tcp_stream
            .set_nonblocking(true)
            .expect("failed to set TCP Stream to non-blocking");
        let mut frame_buf = [0u8; 8];
        let empty = match self.tcp_stream.peek(&mut frame_buf) {
            Ok(_) => false,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => true,
            Err(e) => {
                log::warn!("TCP IO error: {}", e);
                true
            }
        };
        self.tcp_stream
            .set_nonblocking(false)
            .expect("failed to set TCP Stream to non-blocking");
        empty
    }

    /** read all available frames from the websocket and relay each frame to the caller_id */
    fn read(&self) {
        let mut frame_buf = [0u8; WEBSOCKET_PAYLOAD_LEN];
        while let Some(frame) = self
            .framer
            .read_binary(&mut self.ws_stream, &mut frame_buf[..])
            .expect("failed to read websocket")
        {
            let frame: [u8; WEBSOCKET_PAYLOAD_LEN] = frame
                .try_into()
                .expect("websocket frame too large for buffer");
            let buf = Buffer::into_buf(Frame { bytes: frame })
                .expect("failed to serialize websocket frame into buffer");
            buf.send(self.cid, self.opcode)
                .expect("failed to relay websocket frame");
        }
    }
}

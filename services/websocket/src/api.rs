use std::{sync::Arc, time::Duration};


#[allow(dead_code)]
pub(crate) const SERVER_NAME_WEBSOCKET: &str     = "_Websocket Service_";


pub(crate) const KEEPALIVE_TIMEOUT_SECONDS: Duration = Duration::from(55);
pub(crate) const URL_LENGTH_LIMIT: usize = 200;

/// These opcodes can be called by anyone at any time
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub(crate) enum Opcode {
    /// send a KeepAliveRequest
    Tick,
    /// Close Websocket and shutdown server
    Quit,
}

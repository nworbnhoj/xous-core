#[allow(dead_code)]
pub(crate) const SERVER_NAME_WEBSOCKET: &str     = "_Websocket Service_";


/// These opcodes can be called by anyone at any time
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub(crate) enum Opcode {
    Quit,
}

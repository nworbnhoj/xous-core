#[allow(dead_code)]
pub const SERVER_NAME_WEBSOCKET: &str = "_Websocket Service_";

/** limit on the byte length of websocket sub-protocol strings */
pub const SUB_PROTOCOL_LEN: usize = 24;

/// These opcodes can be called by anyone at any time
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum Opcode {
    Open,
}

// a structure for defining the setup of a Websocket.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Debug, Clone, Copy)]
pub struct WebsocketConfig {
    pub sub_protocol: Option<xous_ipc::String<SUB_PROTOCOL_LEN>>,
}

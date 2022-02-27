pub(crate) const SERVER_NAME_ESPEAK: &str     = "_Espeak-ng Server_";

#[allow(dead_code)]
#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub(crate) enum Opcode {
    /// Take a string and plays it on the CODEC
    StrToWav,
}

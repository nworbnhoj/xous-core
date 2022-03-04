#![cfg_attr(target_os = "none", no_std)]
#![allow(nonstandard_style)]

pub type c_char = i8;
pub type c_schar = i8;
pub type c_uchar = u8;
pub type c_short = i16;
pub type c_ushort = u16;
pub type c_int = i32;
pub type c_uint = u32;
pub type c_long = i32;
pub type c_ulong = u32;
pub type c_longlong = i64;
pub type c_ulonglong = u64;
pub type c_float = f32;
pub type c_double = f64;
pub type c_void = core::ffi::c_void;

static mut PUTC_BUF: Vec::<u8> = Vec::new();
#[export_name = "libc_putchar"]
pub unsafe extern "C" fn libc_putchar(
    c: c_char,
) {
    let char = c as u8;
    if char != 0xa && char != 0xd {
        PUTC_BUF.push(char);
    } else {
        let s = String::from_utf8_lossy(&PUTC_BUF);
        log::info!("espeak-ng: {}", s);
    }
}

pub type espeak_ng_STATUS_e = u32;

extern "C" {
    pub fn espeak_ffi_synth(
        text: *const c_char,
        size: c_uint,
    ) -> espeak_ng_STATUS_e;
}


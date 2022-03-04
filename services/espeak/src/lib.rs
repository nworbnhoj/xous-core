#![cfg_attr(target_os = "none", no_std)]

extern crate espeak_sys;

pub mod bindings;
pub use bindings::*;

pub mod api;
use xous::{CID, send_message};
use num_traits::ToPrimitive;

#[derive(Debug)]
pub struct Espeak {
    conn: CID,
}
impl Espeak {
    pub fn new(xns: &xous_names::XousNames) -> Result<Self, xous::Error> {
        REFCOUNT.fetch_add(1, Ordering::Relaxed);
        let conn = xns.request_connection_blocking(api::SERVER_NAME_ESPEAK).expect("Can't connect to Espeak server");
        Ok(Espeak {
            conn
        })
    }
    pub fn test<S: Into<String>>(&self, text: S) -> Result<(), xous::Error> {
        unsafe {
            espeak_ffi_synth(std::ffi::CString::new(text.into())
            .map_err(|_| xous::Error::InternalError)?
            .as_ptr(),
            // obviously, this is a bodge: need to decompose the above pointer thing
            42);
        }
        Ok(())
    }
}

use core::sync::atomic::{AtomicU32, Ordering};
static REFCOUNT: AtomicU32 = AtomicU32::new(0);
impl Drop for Espeak {
    fn drop(&mut self) {
        // the connection to the server side must be reference counted, so that multiple instances of this object within
        // a single process do not end up de-allocating the CID on other threads before they go out of scope.
        // Note to future me: you want this. Don't get rid of it because you think, "nah, nobody will ever make more than one copy of this object".
        if REFCOUNT.fetch_sub(1, Ordering::Relaxed) == 1 {
            unsafe{xous::disconnect(self.conn).unwrap();}
        }
        // if there was object-specific state (such as a one-time use server for async callbacks, specific to the object instance),
        // de-allocate those items here. They don't need a reference count because they are object-specific
    }
}
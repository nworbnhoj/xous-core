use xous::CID;
use xous_ipc::Buffer;
use num_traits::*;
use core::sync::atomic::{AtomicU32, Ordering};
use crate::api::*;

// these exist outside the I2C struct because it needs to synchronize across multiple object instances within the same process
static REFCOUNT: AtomicU32 = AtomicU32::new(0);

#[derive(Debug)]
pub struct I2c {
    conn: CID,
    timeout_ms: u32,
}
impl I2c {
    pub fn new(xns: &xous_names::XousNames) -> Self {
        REFCOUNT.fetch_add(1, Ordering::Relaxed);
        let conn = xns.request_connection_blocking(SERVER_NAME_I2C).expect("Can't connect to I2C");
        I2c {
            conn,
            timeout_ms: 150,
        }
    }

    pub fn i2c_set_timeout(&mut self, timeout: u32) {
        self.timeout_ms = timeout;
    }

    /// initiate an i2c write. This is always a blocking call. In practice, it turns out it's not terribly
    /// useful to just "fire and forget" i2c writes, because actually we cared about the side effect of the
    /// write and don't want execution to move on until the write has been committed,
    /// even if the write "takes a long time"
    pub fn i2c_write(&mut self, dev: u8, adr: u8, data: &[u8]) -> Result<I2cStatus, xous::Error> {
        if data.len() > I2C_MAX_LEN - 1 {
            return Err(xous::Error::OutOfMemory)
        }
        let mut transaction = I2cTransaction::new();

        let mut txbuf = [0; I2C_MAX_LEN];
        txbuf[0] = adr;
        for i in 0..data.len() {
            txbuf[i+1] = data[i];
        }
        transaction.bus_addr = dev;
        transaction.txbuf = Some(txbuf);
        transaction.txlen = (data.len() + 1) as u32;
        transaction.timeout_ms = self.timeout_ms;

        let mut buf = Buffer::into_buf(transaction).or(Err(xous::Error::InternalError))?;
        buf.lend_mut(self.conn, I2cOpcode::I2cTxRx.to_u32().unwrap()).or(Err(xous::Error::InternalError))?;
        let result = buf.to_original::<I2cResult, _>().unwrap();
        match result.status {
            I2cStatus::ResponseWriteOk => {
                Ok(I2cStatus::ResponseWriteOk)
            }
            _ => {
                log::error!("I2C error: {:?}", result);
                Err(xous::Error::InternalError)
            }
        }
    }

    /// initiate an i2c read. if asyncread_cb is `None`, one will be provided and the routine will synchronously block until read is complete.
    /// synchronous reads will return the data in &mut `data`. Asynchronous reads will provide the result in the `rxbuf` field of the `I2cTransaction`
    /// returned via the callback. Note that the callback API may be revised to return a smaller, more targeted structure in the future.
    pub fn i2c_read(&mut self, dev: u8, adr: u8, data: &mut [u8]) -> Result<I2cStatus, xous::Error> {
        if data.len() > I2C_MAX_LEN - 1 {
            return Err(xous::Error::OutOfMemory)
        }
        let mut transaction = I2cTransaction::new();
        let mut txbuf = [0; I2C_MAX_LEN];
        txbuf[0] = adr;
        let rxbuf = [0; I2C_MAX_LEN];
        transaction.bus_addr = dev;
        transaction.txbuf = Some(txbuf);
        transaction.txlen = 1;
        transaction.rxbuf = Some(rxbuf);
        transaction.rxlen = data.len() as u32;
        transaction.timeout_ms = self.timeout_ms;

        let mut buf = Buffer::into_buf(transaction).or(Err(xous::Error::InternalError))?;
        buf.lend_mut(self.conn, I2cOpcode::I2cTxRx.to_u32().unwrap()).or(Err(xous::Error::InternalError))?;
        let result = buf.to_original::<I2cResult, _>().unwrap();
        match result.status {
            I2cStatus::ResponseReadOk => {
                for (&src, dst) in result.rxbuf[..result.rxlen as usize].iter().zip(data.iter_mut()) {
                    *dst = src;
                }
                Ok(I2cStatus::ResponseReadOk)
            }
            _ => {
                log::error!("I2C error: {:?}", result);
                Err(xous::Error::InternalError)
            }
        }
    }
}

impl Drop for I2c {
    fn drop(&mut self) {
        if REFCOUNT.fetch_sub(1, Ordering::Relaxed) == 1 {
            unsafe{xous::disconnect(self.conn).ok();}
        }
    }
}

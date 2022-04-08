use crate::{CommonEnv, ShellCmdApi};
use core::fmt::Write;
use libsignal_service::configuration::SignalServers;
use locales::t;

use url::Url;

use gam::modal::*;
use regex::Regex;

use xous::{MessageEnvelope};
use xous_names::XousNames;
use modals::Modals;

 
use phonenumber::PhoneNumber;
use std::str::FromStr;


#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub(crate) enum PnRendererOpcode {
    PwReturn,
    ModalRedraw,
    ModalKeypress,
    ModalDrop,
    Quit,
}

#[derive(Debug)]
pub struct Register {
    callback_id: Option<u32>,
    callback_conn: u32,
    servers: SignalServers,
    phone_number: Option<PhoneNumber>,
    use_voice_call: bool,
    captcha: Option<Url>,
    force: bool,
}
impl Register {
    pub fn new(xns: &xous_names::XousNames) -> Self {
        let callback_conn = xns
            .request_connection_blocking(crate::SERVER_NAME_SIGNAL)
            .unwrap();
        let url = Url::parse("https://signalcaptchas.org/registration/generate.html")
            .expect("unable to parse url");
        Register {
            callback_id: None,
            callback_conn,
            servers: SignalServers::Staging,
            phone_number: None,
            use_voice_call: false,
            captcha: Some(url),
            force: false,
        }
    }

    fn get_phonenumber(&mut self, modals: &Modals) -> Option<PhoneNumber> {
        let text = modals
            .get_text(
                t!("signal.register.phone.modal.name", xous::LANG),
                Some(Register::phone_number_validator),
                None,
            )
            .expect("Phone Number modal returned no text");

        match PhoneNumber::from_str(text.as_str()) {
            Ok(phone_number) => Some(phone_number),
            Err(e) => None,
        }
    }

    fn phone_number_validator(
        input: TextEntryPayload,
        _opcode: u32,
    ) -> Option<xous_ipc::String<256>> {
        let text_str = input.as_str();
        let re = Regex::new(r"^$|^\+[d]{10,13}$").unwrap();
        if re.is_match(text_str) {
            None
        } else {
            Some(xous_ipc::String::<256>::from_str(
                "enter an valid phone number (+61234567890)",
            ))
        }
    }
    
    
}

impl<'a> ShellCmdApi<'a> for Register {
    cmd_api!(register);

    fn process(
        &mut self,
        _args: xous_ipc::String<1024>,
        env: &mut CommonEnv,
    ) -> Result<Option<xous_ipc::String<1024>>, xous::Error> {
        let mut ret = xous_ipc::String::<1024>::new();
                
        let xns = XousNames::new().unwrap();
        let modals = modals::Modals::new(&xns).unwrap();

        let phone_number = self.get_phonenumber(&modals);
        
  //      modals.show_qrcode("https://signalcaptchas.org/registration/generate.html");
        

        if self.phone_number.is_some() {
            env.manager.register(
                self.servers,
                self.phone_number.clone().unwrap(),
                self.use_voice_call,
                self.captcha.as_ref().map(|u| u.host_str().unwrap()),
                self.force,
            );
            write!(ret, "{}", t!("signal.register.submitted", xous::LANG)).unwrap();
        }

        Ok(Some(ret))
    }

    fn callback(
        &mut self,
        msg: &MessageEnvelope,
        _env: &mut CommonEnv,
    ) -> Result<Option<xous_ipc::String<1024>>, xous::Error> {
        log::info!("received unhandled message {:?}", msg);
        Ok(None)
    }
}

use crate::{ShellCmdApi,CommonEnv};
use core::fmt::Write;
use locales::t;
use libsignal_service::prelude::phonenumber::PhoneNumber;
use libsignal_service::configuration::SignalServers;

use url::Url;

use gam::modal::*;

use graphics_server::api::GlyphStyle;

use regex::Regex;

use xous::{MessageEnvelope, Message};
use xous::{SID, CID};
use xous::{msg_scalar_unpack};
use xous_ipc::Buffer;
use std::sync::Arc;

use core::sync::atomic::{AtomicBool, Ordering};
use num_traits::*;
use std::str::FromStr;

use libsignal_service::prelude::phonenumber::ParseError;


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
        let callback_conn = xns.request_connection_blocking(crate::SERVER_NAME_SIGNAL).unwrap();
        let url = Url::parse("https://signalcaptchas.org/registration/generate.html").expect("unable to parse url");
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



    fn phone_number_ux (&mut self, ux_sid: SID, ux_cid:CID) -> Result<PhoneNumber, ParseError> {
    
       let phone_number_action = TextEntry {
           is_password: false,
           visibility: TextEntryVisibility::LastChars,
           action_conn: ux_cid,
           action_opcode: PnRendererOpcode::PwReturn.to_u32().unwrap(),
           action_payload: TextEntryPayload::new(),
           validator: Some(Register::phone_number_validator),
       };            
       let mut phone_number_modal =
       Modal::new(
           t!("signal.register.phone.modal.name", xous::LANG),
           ActionType::TextEntry(phone_number_action),
           Some(t!("signal.register.phone.modal.top", xous::LANG)),
           Some(t!("signal.register.phone.modal.bottom", xous::LANG)),
           GlyphStyle::Regular,
           8
       );
       phone_number_modal.activate();
       phone_number_modal.spawn_helper(ux_sid, phone_number_modal.sid,
               PnRendererOpcode::ModalRedraw.to_u32().unwrap(),
               PnRendererOpcode::ModalKeypress.to_u32().unwrap(),
               PnRendererOpcode::ModalDrop.to_u32().unwrap(),
       );
   
       let mut phone_str = String::new();
       let renderer_active = Arc::new(AtomicBool::new(false));
       loop {
           let msg = xous::receive_message(ux_sid).unwrap();
           log::debug!("message: {:?}", msg);
           match FromPrimitive::from_usize(msg.body.id()) {
               Some(PnRendererOpcode::PwReturn) => {
                   if renderer_active.load(Ordering::SeqCst) {
                       let buf = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                       let text_entry_payload = buf.to_original::<gam::modal::TextEntryPayload, _>().unwrap();
                       phone_str = text_entry_payload.as_str().to_string();
                   } else {
                       log::warn!("Fat finger event received from Ux, ignoring");
                   }
                   // this resumes the waiting Ux Manager thread
                   renderer_active.store(false, Ordering::SeqCst);
               },
               Some(PnRendererOpcode::ModalRedraw) => {
                   phone_number_modal.redraw();
               },
               Some(PnRendererOpcode::ModalKeypress) => msg_scalar_unpack!(msg, k1, k2, k3, k4, {
                   let keys = [
                       core::char::from_u32(k1 as u32).unwrap_or('\u{0000}'),
                       core::char::from_u32(k2 as u32).unwrap_or('\u{0000}'),
                       core::char::from_u32(k3 as u32).unwrap_or('\u{0000}'),
                       core::char::from_u32(k4 as u32).unwrap_or('\u{0000}'),
                   ];
                   phone_number_modal.key_event(keys);
               }),
               Some(PnRendererOpcode::ModalDrop) => { // this guy should never quit, it's a core OS service
                  panic!("Password modal for PDDB quit unexpectedly");
               },
               Some(PnRendererOpcode::Quit) => {
                   log::warn!("received quit on PDDB password UX renderer loop");
                  xous::return_scalar(msg.sender, 0).unwrap();
                   break;
               },
               None => {
                   log::error!("Couldn't convert opcode: {:?}", msg);
               }
           }
       }
       xous::destroy_server(ux_sid).unwrap();
       
       PhoneNumber::from_str(&phone_str)
   }
   
    
    

   
   fn phone_number_validator(input: TextEntryPayload, _opcode: u32) -> Option<xous_ipc::String::<256>> {
       let text_str = input.as_str();
       let re = Regex::new(r"^\+[d]{10,13}$").unwrap();
       if re.is_match(text_str) {
           None
       } else {
           Some(xous_ipc::String::<256>::from_str("enter an valid phone number (+61234567890)"))
       }
   }
   
    
    
}



impl<'a> ShellCmdApi<'a> for Register {
    cmd_api!(register);

    fn process(&mut self, _args: xous_ipc::String::<1024>, env: &mut CommonEnv) -> Result<Option<xous_ipc::String::<1024>>, xous::Error> {
        let mut ret = xous_ipc::String::<1024>::new();
        
        let renderer_sid: SID = xous::create_server().expect("couldn't create a server for the password UX renderer");
        let renderer_cid: CID = xous::connect(renderer_sid).expect("couldn't connect to the password UX renderer");
      
      
        if self.callback_id.is_none() {
            let cb_id = env.register_handler(xous_ipc::String::<256>::from_str(self.verb()));
            log::trace!("hooking frame callback with ID {}", cb_id);
            env.codec.hook_frame_callback(cb_id, self.callback_conn).unwrap(); // any non-handled IDs get routed to our callback port
            self.callback_id = Some(cb_id);
        }
      
      
        self.phone_number = match Register::phone_number_ux(self, renderer_sid, renderer_cid) {
            Ok (phone_number) => Some(phone_number),
            Err(_e) => {
                log::info!("no valid phone number entered");
                write!(ret, "{}", t!("signal.register.help", xous::LANG)).unwrap();
                None
            }
        };
        
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


    fn callback(&mut self, msg: &MessageEnvelope, _env: &mut CommonEnv) -> Result<Option<xous_ipc::String::<1024>>, xous::Error> {
        log::info!("received unhandled message {:?}", msg);
        Ok(None)
    }
}






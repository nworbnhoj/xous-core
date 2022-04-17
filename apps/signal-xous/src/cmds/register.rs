use crate::{CommonEnv, ShellCmdApi};
use core::fmt::Write;
use libsignal_service::configuration::SignalServers;
use locales::t;

use url::Url;

use gam::modal::*;
use regex::Regex;

use modals::Modals;
use xous::MessageEnvelope;
use xous_names::XousNames;

use phonenumber::PhoneNumber;
use std::str::FromStr;

const VALID_PHONE_NUMBER_REGEX: &str = r"^$|^\+[d]{10,13}$";
const VALID_PWD_LENGTH_REGEX: &str = r"^(.+){6,20}$";
const VALID_PWD_LETTER_REGEX: &str = r"[a-zA-Z]+";
const VALID_PWD_DIGIT_REGEX: &str = r"[0-9]+";
const VALID_PWD_SPECIAL_REGEX: &str = r"[!@#%&]+";
const VALID_VERIFICATION_CODE_REGEX: &str = r"^[\w]{4}$";

const VALIDATION_MODE: [&'static str; 2] = ["sms", "voice"];

enum ValidationMode {
    sms,
    voice,
}

impl ValidationMode {
    fn as_str(&self) -> &str {
        match *self {
            ValidationMode::sms => t!("signal.register.mode.modal.sms", xous::LANG),
            ValidationMode::voice => t!("signal.register.mode.modal.voice", xous::LANG),
        }
    }
}

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
    password: Option<String>,
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
            password: None,
            use_voice_call: false,
            captcha: Some(url),
            force: false,
        }
    }

    fn get_phonenumber(&mut self, modals: &Modals) -> Option<PhoneNumber> {
        let text = modals
            .get_text(
                t!("signal.register.phone.modal.prompt", xous::LANG),
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
        let re = Regex::new(VALID_PHONE_NUMBER_REGEX).unwrap();
        if re.is_match(text_str) {
            None
        } else {
            Some(xous_ipc::String::<256>::from_str(t!(
                "signal.register.password.modal.help",
                xous::LANG
            )))
        }
    }

    fn get_password(&mut self, modals: &Modals) -> Option<String> {
        match modals.get_text(
            t!("signal.register.password.modal.prompt", xous::LANG),
            Some(Register::password_validator),
            None,
        ) {
            Ok(text) => Some(text.as_str().to_string()),
            Err(e) => None,
        }
    }

    fn password_validator(input: TextEntryPayload, _opcode: u32) -> Option<xous_ipc::String<256>> {
        let text_str = input.as_str();
        // rust does not support regex lookaround operators :-(
        if Regex::new(VALID_PWD_LENGTH_REGEX).unwrap().is_match(text_str)
            && Regex::new(VALID_PWD_LETTER_REGEX).unwrap().is_match(text_str)
            && Regex::new(VALID_PWD_DIGIT_REGEX).unwrap().is_match(text_str)
            && Regex::new(VALID_PWD_SPECIAL_REGEX).unwrap()special.is_match(text_str)
        {
            None
        } else {
            Some(xous_ipc::String::<256>::from_str(t!(
                "signal.register.password.modal.help",
                xous::LANG
            )))
        }
    }

    fn solve_captcha(&mut self, modals: &Modals) {
        modals.show_notification(
            t!("signal.register.captcha.modal.prompt", xous::LANG),
            false,
        );
        modals.show_notification(
            "https://signalcaptchas.org/registration/generate.html",
            true,
        );
    }

    fn get_verification_mode(&mut self, modals: &Modals) -> Option<ValidationMode> {
        modals
            .add_list_item(ValidationMode::sms.as_str())
            .expect("failed to add radio item sms");
        modals
            .add_list_item(ValidationMode::voice.as_str())
            .expect("failed to add radio item voice");
        match modals.get_radiobutton(t!("signal.register.mode.modal.prompt", xous::LANG)) {
            Ok(mode) => {
                log::info!("sms:{:?}", mode.as_str().eq(ValidationMode::sms.as_str()));
                log::info!(
                    "voice:{:?}",
                    mode.as_str().eq(ValidationMode::voice.as_str())
                );
                if mode.as_str().eq(ValidationMode::sms.as_str()) {
                    Some(ValidationMode::sms)
                } else if mode.as_str().eq(ValidationMode::voice.as_str()) {
                    Some(ValidationMode::voice)
                } else {
                    None
                }
            }
            Err(e) => None,
        }
    }

    fn get_verification_code(&mut self, modals: &Modals) -> Option<String> {
        match modals.get_text(
            t!("signal.register.validation.modal.prompt", xous::LANG),
            Some(Register::verification_code_validator),
            None,
        ) {
            Ok(text) => Some(text.as_str().to_string()),
            Err(_) => None,
        }
    }

    fn verification_code_validator(
        input: TextEntryPayload,
        _opcode: u32,
    ) -> Option<xous_ipc::String<256>> {
        let text_str = input.as_str();
        let re = Regex::new(VALID_VERIFICATION_CODE_REGEX).unwrap();
        if re.is_match(text_str) {
            None
        } else {
            Some(xous_ipc::String::<256>::from_str(
                "4 characters received from Signal",
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
        let fail = Ok(Some(xous_ipc::String::<1024>::from_str(t!(
            "signal.register.fail",
            xous::LANG
        ))));
        let success = Ok(Some(xous_ipc::String::<1024>::from_str(t!(
            "signal.register.success",
            xous::LANG
        ))));

        let xns = XousNames::new().unwrap();
        let modals = modals::Modals::new(&xns).unwrap();
        /*
                let phone_number = match self.get_phonenumber(&modals) {
                    Some(number) => {
                        log::info!("valid phone number: {:?}", number);
                        number
                    }
                    None => {
                        log::info!("invalid phone number");
                        return fail
                    }
                };
        */
        let password = match self.get_password(&modals) {
            Some(pwd) => {
                log::info!("valid password");
                pwd
            }
            None => {
                log::info!("invalid password.");
                return fail;
            }
        };
        let captcha = self.solve_captcha(&modals);

        let use_voice_call = match self.get_verification_mode(&modals) {
            Some(mode) => {
                log::info!("valid verification mode: {}", mode.as_str());
                match mode {
                    ValidationMode::sms => false,
                    ValidationMode::voice => true,
                }
            }
            None => {
                log::info!("invalid verification mode");
                return fail;
            }
        };

        // request voice verification

        let verification_code = match self.get_verification_code(&modals) {
            Some(code) => {
                log::info!("valid verification_code");
                code
            }
            None => {
                log::info!("invalid verification_code");
                return fail;
            }
        };

        success
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

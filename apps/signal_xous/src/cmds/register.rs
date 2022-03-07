use crate::{ShellCmdApi,CommonEnv};
use xous_ipc::String;
use core::fmt::Write;
use locales::t;
use regex::Regex;
use directories::ProjectDirs;
use presage::{
    prelude::{
        proto::sync_message:: SignalServers,
    },
    prelude::{phonenumber::PhoneNumber},
    Manager, SledConfigStore,
};
use structopt::StructOpt;
use url::Url;


#[derive(Debug)]
pub struct Register {
    #[structopt(long = "servers", short = "s", default_value = "staging")]
    servers: SignalServers,
    #[structopt(long, help = "Phone Number to register with in E.164 format")]
    phone_number: PhoneNumber,
    #[structopt(long)]
    use_voice_call: bool,
    #[structopt(
        long = "captcha",
        help = "Captcha obtained from https://signalcaptchas.org/registration/generate.html"
    )]
    captcha: Option<Url>,
    #[structopt(long, help = "Force to register again if already registered")]
    force: bool,
}
impl Register {
    pub fn new(xns: &xous_names::XousNames) -> Self {
        Register {
            servers: "staging",
            phone_number: None,
            use_voice_call: false,
            captcha: None,
            force: false,
        }
    }
}


impl<'a> ShellCmdApi<'a> for Register {
    cmd_api!(register);

    fn process(&mut self, args: String::<1024>, env: &mut CommonEnv) -> Result<Option<String::<1024>>, xous::Error> {
        let mut ret = String::<1024>::new();
        let helpstring = t!("signal.register.help", xous::LANG);
        let mut tokens = args.as_str().unwrap().split(' ');

        if let Some(sub_cmd) = tokens.next() {
            match sub_cmd {
                "servers" => {}
                "use_voice_call" => {}
                "captcha" => {}
                "force" => {}
                _ => {
                    self.phone_number = if let Some(phone_str) = tokens.next() {
                        let re = Regex::new("^+[0-9]{9,12}$").unwrap();
                        if re.is_match(phone_str) {
                            PhoneNumber(phone_str)
                        } else {
                            None
                        }
                    }
                }
            }
            if self.phone_number {

                let db_path = args.db_path.unwrap_or_else(|| {
                    ProjectDirs::from("org", "xous", "signal")
                        .unwrap()
                        .config_dir()
                        .into()
                    });
                let config_store = SledConfigStore::new(db_path)?;
                let csprng = rand::thread_rng();
                let mut manager = Manager::new(config_store, csprng)?;

                manager
                    .register(
                        self.servers,
                        self.phone_number,
                        self.use_voice_call,
                        self.captcha.as_ref().map(|u| u.host_str().unwrap()),
                        self.force,
                    )
                    .await?; 
                write!(ret, "{}", t!("signal.register.submitted", xous::LANG)).unwrap();
            } else {
                write!(ret, "{}", helpstring).unwrap();
            }
        }
        Ok(Some(ret))
    }

}

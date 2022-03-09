use crate::{ShellCmdApi,CommonEnv};
use xous_ipc::String;
use core::fmt::Write;
use locales::t;
use directories::ProjectDirs;
use presage::{
    prelude::{
        proto::sync_message:: SignalServers,
    },
    prelude::Manager,
};
use structopt::StructOpt;


#[derive(Debug)]
pub struct Verify {
    #[structopt(long, short = "c", help = "SMS / Voice-call confirmation code")]
    confirmation_code: u32,
}
    
impl Verify {
    pub fn new(xns: &xous_names::XousNames) -> Self {
        Register {
            servers: "staging",
            device_name: "precursor",
        }
    }
}


impl<'a> ShellCmdApi<'a> for LinkDevice {
    cmd_api!(link_device);

    fn process(&mut self, args: String::<1024>, env: &mut CommonEnv) -> Result<Option<String::<1024>>, xous::Error> {
        let mut ret = String::<1024>::new();
        let helpstring = t!("signal.link_device.help", xous::LANG);
        let mut tokens = args.as_str().unwrap().split(' ');        
        
        self.confirmation_code = if let Some(code_str) = tokens.next() {
            let re = Regex::new("^+{9,12}$").unwrap();
            if re.is_match(code_str) {
                self.confirmation_code = code_str;
                let config_store = SledConfigStore::new(db_path)?;
                let csprng = rand::thread_rng();
                let mut manager = Manager::new(config_store, csprng)?;
                manager.confirm_verification_code(confirmation_code).await?; 
                write!(ret, "{}", t!("signal.verify.submitted", xous::LANG)).unwrap();
            } else {
                write!(ret, "{}", helpstring).unwrap();
            }
        } else {
            write!(ret, "{}", helpstring).unwrap();
        }
        Ok(Some(ret))
    }
}

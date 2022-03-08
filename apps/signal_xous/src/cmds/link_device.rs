use crate::{ShellCmdApi,CommonEnv};
use xous_ipc::String;
use core::fmt::Write;
use locales::t;
use directories::ProjectDirs;
use presage::{
    prelude::{
        proto::sync_message:: SignalServers,
    },
    prelude::{phonenumber::PhoneNumber},
    Manager, SledConfigStore,
};
use structopt::StructOpt;
//use curve25519-dalek::*;


#[derive(Debug)]
pub struct LinkDevice {
    /// Possible values: staging, production
    #[structopt(long, short = "s", default_value = "staging")]
    servers: SignalServers,
    #[structopt(
        long,
        short = "n",
        help = "Name of the device to register in the primary client"
    )]
    device_name: String,
}

impl LinkDevice {
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

        if let Some(sub_cmd) = tokens.next() {
            match sub_cmd {
                "servers" => {}
                "device_name" => {}
            }
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
               .link_device(
                        self.servers,
                        self.device_name,
                )
                .await?; 
            write!(ret, "{}", t!("signal.link_device.submitted", xous::LANG)).unwrap();
        } else {
            write!(ret, "{}", helpstring).unwrap();
        }
        Ok(Some(ret))
    }

}

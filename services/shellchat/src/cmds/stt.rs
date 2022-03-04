use crate::{ShellCmdApi, CommonEnv};
use xous_ipc::String;
use espeak::Espeak;

#[derive(Debug)]
pub struct Stt {
    pub espeak: Espeak,
}
impl Stt {
    pub fn new(xns: &xous_names::XousNames) -> Stt {
        Stt {
            espeak: Espeak::new(xns).unwrap(),
        }
    }
}

impl<'a> ShellCmdApi<'a> for Stt {
    cmd_api!(Stt); // inserts boilerplate for command API

    fn process(&mut self, args: String::<1024>, env: &mut CommonEnv) -> Result<Option<String::<1024>>, xous::Error> {
        use core::fmt::Write;
        let mut ret = String::<1024>::new();
        let helpstring = "Stt options: test";

        let mut tokens = args.as_str().unwrap().split(' ');

        if let Some(sub_cmd) = tokens.next() {
            match sub_cmd {
                "test" => {
                    self.espeak.test("this is a test of espeak integration").unwrap();
                }
                _ => {
                    write!(ret, "{}", helpstring).unwrap();
                }
            }

        } else {
            write!(ret, "{}", helpstring).unwrap();
        }
        Ok(Some(ret))
    }
}

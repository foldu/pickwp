use oneshot_reqrep::Req;
use serde::{Deserialize, Serialize};
use structopt::StructOpt;

pub const SOCK_PATH: &str = "\0pickwp";

#[derive(StructOpt, Debug, Serialize, Deserialize, Copy, Clone)]
pub enum Command {
    #[structopt(name = "refresh")]
    Refresh,

    #[structopt(name = "rescan")]
    Rescan,
}

impl Req for Command {
    type Rep = ();
}

use std::collections::HashMap;

use oneshot_reqrep::Req;
use serde::{Deserialize, Serialize};
use structopt::StructOpt;

// FIXME: Use a proper ipc mechanism that hopefully works with std::future

pub const SOCK_PATH: &str = "\0pickwp";

#[derive(StructOpt, Debug, Serialize, Deserialize, Copy, Clone)]
pub enum Command {
    Refresh,

    Rescan,

    ReloadConfig,

    Current,
}

#[derive(Serialize, Deserialize)]
pub enum Reply {
    Unit,
    Wps(HashMap<String, Option<String>>),
}

impl Req for Command {
    type Rep = Result<Reply, String>;
}

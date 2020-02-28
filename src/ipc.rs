use crate::{cli::Command, config};
use oneshot_reqrep::Req;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// FIXME: Use a proper ipc mechanism that hopefully works with std::future

pub const SOCK_PATH: &str = "\0pickwp";

#[derive(Serialize, Deserialize)]
pub enum Reply {
    Unit,
    Wps(HashMap<String, Option<String>>),
    Filters(Vec<config::Filter>),
    FreezeStatus(bool),
}

impl Req for Command {
    type Rep = Result<Reply, String>;
}

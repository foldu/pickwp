use std::collections::HashMap;

use oneshot_reqrep::Req;
use serde::{Deserialize, Serialize};
use structopt::StructOpt;

use crate::config;

// FIXME: Use a proper ipc mechanism that hopefully works with std::future

pub const SOCK_PATH: &str = "\0pickwp";

#[derive(StructOpt, Debug, Serialize, Deserialize, Copy, Clone)]
pub enum Command {
    /// Select some new wallpapers
    Refresh,

    /// Rescan wallpaper directory
    Rescan,

    /// Reload config file
    ReloadConfig,

    /// Print currently selected wallpapers
    Current,

    /// Print active filters
    Filters,
}

#[derive(Serialize, Deserialize)]
pub enum Reply {
    Unit,
    Wps(HashMap<String, Option<String>>),
    Filters(Vec<config::Filter>),
}

impl Req for Command {
    type Rep = Result<Reply, String>;
}

use serde::{Deserialize, Serialize};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
pub struct Opt {
    #[structopt(subcommand)]
    pub cmd: Option<Command>,
    #[structopt(flatten)]
    pub cmd_config: CmdConfig,
}

#[derive(StructOpt, Debug)]
pub struct CmdConfig {
    /// Format output in json
    #[structopt(short, long)]
    pub json: bool,
}

#[derive(StructOpt, Debug, Serialize, Deserialize)]
pub enum Command {
    /// Select some new wallpapers
    Refresh,

    /// Rescan wallpaper directory
    Rescan,

    /// Print currently selected wallpapers
    Current,

    /// Print active filters
    Filters {
        #[structopt(subcommand)]
        action: Option<FilterCommand>,
    },

    /// Stop changing current wallpapers
    ToggleFreeze,
}

#[derive(StructOpt, Debug, Serialize, Deserialize)]
pub enum FilterCommand {
    Rm { id: usize },
    // FIXME: reenable this
    //Add {
    //    #[structopt(required = true)]
    //    filters: Vec<config::Filter>,
    //},
}


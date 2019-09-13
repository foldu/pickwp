use cfgen::{prelude::*, ExpandedPath};
use serde::Deserialize;

use crate::filter::{FilenameFilter, TimeFilter};

const DEFAULT: &str = include_str!("../default_config.yml");

#[derive(Deserialize, Copy, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Fill,

    Tile,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            Mode::Fill => "fill",
            Mode::Tile => "tile",
        };
        fmt.write_str(s)
    }
}

#[derive(Cfgen, Debug, Deserialize)]
#[cfgen(default = "DEFAULT")]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub wp_dir: ExpandedPath,
    pub mode: Mode,
    pub rescan_interval: u64,
    pub refresh_interval: u64,
    pub filters: Vec<Filter>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum Filter {
    LastShown,

    FileTime(TimeFilter),

    Filename(FilenameFilter),
}

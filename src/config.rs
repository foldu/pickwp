use cfgen::{prelude::*, ExpandedPath};
use serde::{Deserialize, Serialize};

use crate::{
    filter::{FilenameFilter, TimeFilter},
    monitor::Mode,
};

const DEFAULT: &str = include_str!("../default_config.yml");

#[derive(Cfgen, Debug, Deserialize)]
#[cfgen(default = "DEFAULT")]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub wp_dir: ExpandedPath,
    pub backend: Backend,
    pub mode: Mode,
    pub rescan_interval: u64,
    pub refresh_interval: u64,
    pub filters: Vec<Filter>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum Filter {
    LastShown,

    FileTime(TimeFilter),

    Filename(FilenameFilter),
}

impl std::str::FromStr for Filter {
    type Err = serde_yaml::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_yaml::from_str(s)
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Sway,
}

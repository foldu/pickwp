use cfgen::{prelude::*, ExpandedPath};
use serde::Deserialize;

use crate::filter::TimeFilter;

const DEFAULT: &str = include_str!("../default_config.yml");

#[derive(Cfgen, Debug, Deserialize)]
#[cfgen(default = "DEFAULT")]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub wp_dir: ExpandedPath,
    pub filters: Vec<Filter>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Filter {
    LastShown,

    FileTime(TimeFilter),
}

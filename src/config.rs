use crate::{
    filter::{FilenameFilter, TagFilter, TimeFilter},
    monitor::Mode,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::path::PathBuf;

const DEFAULT: &str = include_str!("../default_config.yml");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(deserialize_with = "cfgen::expandpath")]
    pub wp_dir: PathBuf,
    pub backend: Backend,
    pub mode: Mode,
    pub rescan_interval: u64,
    pub refresh_interval: u64,
    pub filters: Vec<Filter>,
}
pub static CONFIG_PATH: Lazy<PathBuf> = Lazy::new(|| {
    directories::ProjectDirs::from("org", "foldu", env!("CARGO_PKG_NAME"))
        .unwrap()
        .config_dir()
        .join("config.yml")
});

impl Config {
    pub fn load_from_buf(buf: &[u8]) -> Result<Self, Error> {
        serde_yaml::from_slice(buf).context(Yaml)
    }

    pub async fn load_or_write_default() -> Result<Self, Error> {
        match Self::load().await {
            Err(Error::Read { source }) if source.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir_all(&*CONFIG_PATH.parent().unwrap())
                    .await
                    .context(WriteDefault)?;
                tokio::fs::write(&*CONFIG_PATH, DEFAULT.as_bytes())
                    .await
                    .context(WriteDefault)?;
                Self::load().await
            }
            a => a,
        }
    }

    pub async fn load() -> Result<Self, Error> {
        let buf = tokio::fs::read(&*CONFIG_PATH).await.context(Read)?;
        Self::load_from_buf(&buf)
    }
}

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    #[snafu(display("Can't deserialize config: {}", source))]
    Yaml { source: serde_yaml::Error },

    #[snafu(display("Can't read config in {}: {}", CONFIG_PATH.display(), source))]
    Read { source: std::io::Error },

    #[snafu(display("Can't write default config to {}: {}", CONFIG_PATH.display(), source))]
    WriteDefault { source: std::io::Error },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum Filter {
    LastShown,

    FileTime(TimeFilter),

    Filename(FilenameFilter),

    Tag(TagFilter),
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

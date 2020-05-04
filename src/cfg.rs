use crate::monitor::Mode;
use serde::{Deserialize, Deserializer};
use snafu::ResultExt;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use time::{Date, OffsetDateTime, UtcOffset};

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub wp_dir: PathBuf,
    //pub backend: Backend,
    pub mode: Mode,
    //pub time: TimeKind,
    #[serde(with = "humantime_serde")]
    pub rescan_interval: Duration,
    #[serde(with = "humantime_serde")]
    pub refresh_interval: Duration,
    pub filter: Filter,
}

const DEFAULT_CONFIG: &str = include_str!("../default_config.toml");

impl Config {
    fn load(path: &Path) -> Result<Self, Error> {
        fs::read(&path)
            .context(Read { path })
            .and_then(|buf| Self::from_slice(&buf))
    }

    pub fn load_or_write_default(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        match fs::metadata(&path) {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&path, DEFAULT_CONFIG).with_context(|| Read { path: path.clone() })?;
            }
            Err(source) => {
                return Err(Error::Read {
                    source,
                    path: path.to_owned(),
                })
            }
        };

        Self::load(path)
    }

    pub fn from_slice(slice: &[u8]) -> Result<Self, Error> {
        toml::from_slice(slice).context(Toml)
    }
}

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    Toml {
        source: toml::de::Error,
    },
}

fn deserialize_opt_date<'de, D>(de: D) -> Result<Option<OffsetDateTime>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(de)?;

    opt.map(|s| {
        Date::parse(&s, "%F")
            .map(|d| {
                d.midnight()
                    .assume_offset(UtcOffset::current_local_offset())
            })
            .map_err(|e| serde::de::Error::custom(e))
    })
    .transpose()
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Filter {
    pub last_shown: bool,
    pub tags: Vec<String>,
    #[serde(deserialize_with = "deserialize_opt_date")]
    pub from_time: Option<OffsetDateTime>,
    #[serde(deserialize_with = "deserialize_opt_date")]
    pub to_time: Option<OffsetDateTime>,
}

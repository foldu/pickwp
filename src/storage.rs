use std::{
    convert::{TryFrom, TryInto},
    fs::Metadata,
    os::unix::prelude::*,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SecondaryMap, SlotMap};
use snafu::Snafu;

use crate::util::PathBufExt;

new_key_type! {
    pub struct FileKey;
}

bitflags! {
    pub struct StorageFlags: u8 {
        const NONE = 0;
        const RELAPATH = 1 << 0;
        const FILETIME = 1 << 1;
    }
}

#[derive(Debug)]
pub struct Storage {
    pub relative_paths: SlotMap<FileKey, RelativePath>,
    pub times: SecondaryMap<FileKey, Time>,
}

impl Storage {
    pub fn new() -> Self {
        Self {
            relative_paths: SlotMap::with_key(),
            times: SecondaryMap::new(),
        }
    }

    pub fn refresh<I>(&mut self, it: I)
    where
        I: IntoIterator<Item = (RelativePath, Option<Time>)>,
    {
        self.relative_paths.clear();
        self.times.clear();

        for (path, time) in it {
            let key = self.relative_paths.insert(path);
            if let Some(time) = time {
                self.times.insert(key, time);
            }
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = FileKey> + '_ {
        self.relative_paths.keys()
    }
}

#[derive(Debug)]
pub struct Time {
    atime: SystemTime,
    mtime: SystemTime,
    ctime: SystemTime,
}

impl Time {
    pub fn select(&self, kind: TimeKind) -> SystemTime {
        match kind {
            TimeKind::Mtime => self.mtime,
            TimeKind::Atime => self.atime,
            TimeKind::Ctime => self.ctime,
        }
    }

    pub fn from_meta(meta: &Metadata) -> Self {
        Self {
            atime: meta.accessed().unwrap(),
            ctime: SystemTime::UNIX_EPOCH
                + Duration::from_secs(
                    meta.ctime()
                        .try_into()
                        .expect("Overflow when trying to get ctime"),
                ),
            mtime: meta.modified().unwrap(),
        }
    }
}

#[derive(Debug)]
pub struct RelativePath(String);

#[derive(Snafu, Debug)]
pub enum RelativePathError {
    #[snafu(display("File path is not relative"))]
    NotRelative,
    #[snafu(display("File path is not UTF-8"))]
    InvalidUTF8,
}

impl TryFrom<PathBuf> for RelativePath {
    type Error = RelativePathError;
    fn try_from(other: PathBuf) -> Result<Self, Self::Error> {
        if other.is_relative() {
            other
                .into_string()
                .map_err(|_| RelativePathError::InvalidUTF8)
                .map(Self)
        } else {
            Err(RelativePathError::NotRelative)
        }
    }
}

impl std::ops::Deref for RelativePath {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Deserialize, Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimeKind {
    Mtime,
    Atime,
    Ctime,
}

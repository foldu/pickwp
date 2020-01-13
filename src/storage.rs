use crate::{
    cache::{self, Cache},
    util::PathBufExt,
};
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SecondaryMap, SlotMap};
use snafu::Snafu;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    convert::{TryFrom, TryInto},
    fs::Metadata,
    os::unix::prelude::*,
    path::PathBuf,
    time::{Duration, SystemTime},
};

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

pub struct Storage {
    pub paths: SlotMap<FileKey, ()>,
    pub relative_paths: SecondaryMap<FileKey, RelativePath>,
    pub times: SecondaryMap<FileKey, Time>,
    pub tags: SecondaryMap<FileKey, BTreeSet<i32>>,
    pub relative_keys: HashMap<RelativePath, FileKey>,
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            paths: SlotMap::with_key(),
            relative_paths: SecondaryMap::new(),
            times: SecondaryMap::new(),
            tags: SecondaryMap::new(),
            relative_keys: HashMap::new(),
        }
    }
}

impl Storage {
    pub fn refresh<I>(&mut self, it: I, cache: &Cache) -> Result<(), cache::Error>
    where
        I: IntoIterator<Item = (RelativePath, Time)>,
    {
        let mut unvisited = self.paths.keys().collect::<HashSet<_>>();

        for (path, time) in it {
            match self.relative_keys.get(&path) {
                Some(key) => {
                    unvisited.remove(key);
                }
                None => {
                    let key = self.paths.insert(());
                    let tags = cache.get_path_tags(&path)?;
                    self.tags.insert(key, tags);
                    self.relative_keys.insert(path.clone(), key);
                    self.relative_paths.insert(key, path);
                    self.times.insert(key, time);
                }
            }
        }

        println!("{:#?}", unvisited);

        for key in unvisited {
            self.times.remove(key);
            self.tags.remove(key);
            if let Some(rela_path) = self.relative_paths.remove(key) {
                self.relative_keys.remove(&rela_path);
            }
        }

        Ok(())
    }

    pub fn keys(&self) -> impl Iterator<Item = FileKey> + '_ {
        self.paths.keys()
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

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
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

impl AsRef<str> for RelativePath {
    fn as_ref(&self) -> &str {
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

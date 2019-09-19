use std::{collections::HashSet, time::SystemTime};

use serde::{Deserialize, Serialize};

use crate::{
    config,
    storage::{FileKey, Storage, StorageFlags, TimeKind},
};

pub trait Filter {
    fn after_wp_refresh(&mut self, _: &[FileKey]) {}

    fn is_ok(&mut self, id: FileKey, storage: &Storage) -> bool;

    fn needed_storages(&self) -> StorageFlags {
        StorageFlags::NONE
    }

    fn serializeable(&self) -> config::Filter;
}

#[derive(Default)]
pub struct LastShown {
    last: HashSet<FileKey>,
}

impl Filter for LastShown {
    fn after_wp_refresh(&mut self, new_wps: &[FileKey]) {
        self.last.clear();
        for wp in new_wps {
            self.last.insert(*wp);
        }
    }

    fn is_ok(&mut self, id: FileKey, _storage: &Storage) -> bool {
        !self.last.contains(&id)
    }

    fn serializeable(&self) -> config::Filter {
        config::Filter::LastShown
    }
}

impl From<config::Filter> for Box<dyn Filter> {
    fn from(other: config::Filter) -> Self {
        match other {
            config::Filter::LastShown => Box::new(LastShown::default()),
            config::Filter::FileTime(filter) => Box::new(filter),
            config::Filter::Filename(filter) => Box::new(filter),
        }
    }
}

#[derive(Deserialize, Debug, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct TimeFilter {
    time_kind: TimeKind,
    #[serde(default)]
    #[serde(with = "humantime_serde")]
    from: Option<SystemTime>,
    #[serde(default)]
    #[serde(with = "humantime_serde")]
    to: Option<SystemTime>,
}

impl Filter for TimeFilter {
    fn needed_storages(&self) -> StorageFlags {
        StorageFlags::FILETIME
    }

    fn is_ok(&mut self, id: FileKey, storage: &Storage) -> bool {
        let time = storage.times.get(id).unwrap().select(self.time_kind);

        match (self.from, self.to) {
            (Some(from), Some(to)) => time >= from && time <= to,
            (Some(from), None) => time >= from,
            (None, Some(to)) => time <= to,
            _ => true,
        }
    }

    fn serializeable(&self) -> config::Filter {
        config::Filter::FileTime(self.clone())
    }
}

#[derive(Deserialize, Debug, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct FilenameFilter {
    contains: String,
}

impl Filter for FilenameFilter {
    fn needed_storages(&self) -> StorageFlags {
        StorageFlags::RELAPATH
    }

    fn is_ok(&mut self, id: FileKey, storage: &Storage) -> bool {
        let path = storage.relative_paths.get(id).unwrap();
        path.contains(&self.contains)
    }

    fn serializeable(&self) -> config::Filter {
        config::Filter::Filename(self.clone())
    }
}

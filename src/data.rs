use crate::db::RootId;
use std::{convert::TryFrom, os::unix::prelude::*, path::PathBuf, time::SystemTime};
use time::OffsetDateTime;

#[derive(Debug, Clone, Hash, PartialEq, Eq, derive_more::Deref, derive_more::AsRef)]
pub struct RelativePath(String);

#[derive(snafu::Snafu, Debug)]
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
            String::from_utf8(other.into_os_string().into_vec())
                .map_err(|_| RelativePathError::InvalidUTF8)
                .map(Self)
        } else {
            Err(RelativePathError::NotRelative)
        }
    }
}

impl TryFrom<String> for RelativePath {
    type Error = RelativePathError;
    fn try_from(other: String) -> Result<Self, Self::Error> {
        Self::try_from(PathBuf::from(other))
    }
}

#[derive(Eq, PartialEq, Debug, sqlx::Type, Clone, Copy)]
#[sqlx(transparent)]
pub struct UnixTimestamp(i64);

impl From<i64> for UnixTimestamp {
    fn from(other: i64) -> Self {
        Self(other)
    }
}

impl From<SystemTime> for UnixTimestamp {
    fn from(other: SystemTime) -> Self {
        Self(
            other
                .duration_since(std::time::UNIX_EPOCH)
                // NOTE: impossible to panic because UNIX_EPOCH is 0
                .unwrap()
                // NOTE: statx/stat64 returns i64s so `as` doesn't matter
                .as_secs() as i64,
        )
    }
}

impl From<OffsetDateTime> for UnixTimestamp {
    fn from(other: OffsetDateTime) -> Self {
        Self(other.timestamp())
    }
}

#[derive(Eq, PartialEq, Debug)]
pub struct Time {
    pub mtime: UnixTimestamp,
    pub btime: Option<UnixTimestamp>,
}

#[derive(Debug)]
pub struct PathData {
    pub root_id: RootId,
    pub path: RelativePath,
    pub time: Time,
}

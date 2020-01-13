use crate::storage::RelativePath;
use rusqlite::OptionalExtension;
use snafu::ResultExt;
use std::{collections::BTreeSet, path::PathBuf};
use tgcd::Tag;

pub struct Cache(rusqlite::Connection);

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    #[snafu(display("Can't create cache dir {}: {}", path.display(), source))]
    CreateDbDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("Can't open cache in {}: {}", path.display(), source))]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },

    #[snafu(display("Failed to run migrations: {}", source))]
    #[snafu(context(false))]
    RunMigrations { source: refinery_migrations::Error },

    #[snafu(display("Db operation failed: {}", source))]
    #[snafu(context(false))]
    Sqlite { source: rusqlite::Error },
}

mod embedded {
    refinery::embed_migrations!("migrations");
}

impl Cache {
    pub fn open() -> Result<Self, Error> {
        let path = directories::ProjectDirs::from("org", "foldu", env!("CARGO_PKG_NAME"))
            .unwrap()
            .cache_dir()
            .join("cache.sqlite");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(&parent).with_context(|| CreateDbDir {
            path: parent.to_owned(),
        })?;

        let mut cxn = rusqlite::Connection::open(&path).with_context(|| Open { path })?;

        embedded::migrations::runner().run(&mut cxn)?;

        Ok(Self(cxn))
    }

    pub fn transaction(&mut self) -> Result<CacheTransaction<'_>, Error> {
        self.0
            .transaction()
            .map_err(Error::from)
            .map(CacheTransaction)
    }

    pub fn get_path_tags(&self, path: &RelativePath) -> Result<BTreeSet<i32>, Error> {
        let mut stmnt = self.0.prepare_cached(
            "\
                SELECT tag_id FROM relative_path
                INNER JOIN path_tag ON relative_path_id = id
                WHERE file_path = ?",
        )?;

        let ret = stmnt
            .query_map(&[path.as_ref()], |row| row.get(0))?
            .map(|res| res.map_err(Error::from))
            .collect::<Result<_, _>>()?;

        Ok(ret)
    }

    pub fn get_tag_id(&self, tag: &str) -> Result<Option<i32>, Error> {
        let mut stmnt = self.0.prepare_cached("SELECT id FROM tag WHERE name = ?")?;
        stmnt
            .query_row(&[tag], |row| row.get(0))
            .optional()
            .map_err(Error::from)
    }

    pub fn path_exists(&self, path: &RelativePath) -> Result<bool, Error> {
        let mut stmnt = self
            .0
            .prepare_cached("SELECT 1 FROM relative_path WHERE file_path = ?")?;

        Ok(stmnt
            .query_row(&[path.as_ref()], |row| row.get::<_, i32>(0))
            .optional()?
            .is_some())
    }
}

pub struct CacheTransaction<'a>(rusqlite::Transaction<'a>);

#[derive(Copy, Clone)]
struct PathId(i32);

#[derive(Copy, Clone)]
struct TagId(i32);

impl<'a> CacheTransaction<'a> {
    pub fn insert_path_with_tags(
        &self,
        relative_path: &RelativePath,
        tags: &[Tag],
    ) -> Result<(), Error> {
        let path_id = self.get_or_insert_relative_path(relative_path)?;
        let tags = self.get_or_insert_tags(tags)?;
        self.associate_relative_path_with_tags(path_id, &tags)?;
        Ok(())
    }

    pub fn commit(self) -> Result<(), Error> {
        self.0.commit()?;
        Ok(())
    }

    fn associate_relative_path_with_tags(
        &self,
        path_id: PathId,
        tags: &[TagId],
    ) -> Result<(), Error> {
        let mut stmnt = self
            .0
            .prepare_cached("INSERT INTO path_tag(relative_path_id, tag_id) VALUES ($1, $2)")?;
        tags.iter()
            .map(|tag| {
                stmnt
                    .execute(&[path_id.0, tag.0])
                    .map_err(Error::from)
                    .map(|_| ())
            })
            .collect()
    }

    fn get_or_insert_tags(&self, tags: &[impl AsRef<str>]) -> Result<Vec<TagId>, Error> {
        let mut insert_stmnt = self
            .0
            .prepare_cached("INSERT OR IGNORE INTO tag(name) VALUES($1)")?;
        let mut get_stmnt = self
            .0
            .prepare_cached("SELECT id FROM tag WHERE name = $1")?;

        tags.iter()
            .map(|tag| {
                insert_stmnt.execute(&[tag.as_ref()])?;
                get_stmnt
                    .query_row(&[tag.as_ref()], |row| row.get(0))
                    .map_err(Error::from)
                    .map(TagId)
            })
            .collect()
    }

    fn get_or_insert_relative_path(&self, relative_path: &str) -> Result<PathId, Error> {
        let mut stmnt = self
            .0
            .prepare_cached("INSERT OR IGNORE INTO relative_path(file_path) VALUES($1)")?;
        stmnt.execute(&[relative_path])?;
        let mut stmnt = self
            .0
            .prepare_cached("SELECT id FROM relative_path WHERE file_path = $1")?;
        stmnt
            .query_row(&[relative_path], |row| row.get(0))
            .map_err(Error::from)
            .map(PathId)
    }
}

use snafu::ResultExt;
use std::path::PathBuf;

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
    RunMigrations { source: refinery_migrations::Error },

    #[snafu(display("Db operation failed: {}", source))]
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
        let parent = std::fs::create_dir_all(&parent).with_context(|| CreateDbDir {
            path: parent.to_owned(),
        })?;

        let mut cxn = rusqlite::Connection::open(&path).with_context(|| Open { path })?;

        embedded::migrations::runner()
            .run(&mut cxn)
            .context(RunMigrations)?;

        Ok(Self(cxn))
    }

    pub fn transaction(&mut self) -> Result<CacheTransaction<'_>, Error> {
        self.0.transaction().context(Sqlite).map(CacheTransaction)
    }
}

pub struct CacheTransaction<'a>(rusqlite::Transaction<'a>);

impl<'a> CacheTransaction<'a> {
    pub fn get_or_insert_tag(&mut self, tag: &str) -> Result<i32, Error> {
        let stmnt = self
            .0
            .prepare_cached("INSERT OR IGNORE INTO tag(name) VALUES($1)")
            .context(Sqlite)?;
        stmnt.execute(&[tag]);
        //self.0.execute(stmnt, &[tag]).context(Sqlite)?;
        let stmnt = self
            .0
            .prepare_cached("SELECT id FROM tag WHERE name = $1")
            .context(Sqlite)?;
        stmnt.query_row(&[tag], |row| row.get(0)).context(Sqlite)
    }
}

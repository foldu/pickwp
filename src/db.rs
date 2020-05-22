use crate::{
    cfg::Filter,
    data::{PathData, RelativePath, Time, UnixTimestamp},
};
use snafu::ResultExt;
use sqlx::{prelude::*, sqlite::SqliteRow, SqliteConnection};
use std::{
    convert::TryFrom,
    path::{Path, PathBuf},
};
use tgcd::Tag;
use tokio::fs;

#[derive(snafu::Snafu, Debug)]
pub enum OpenError {
    #[snafu(display("Cache path is not valid UTF-8 (fix your XDG_CACHE_DIR)"))]
    CachePathNotUtf8 { path: PathBuf },

    #[snafu(display("Can't open database"))]
    OpenDb { source: sqlx::Error },

    #[snafu(display("Could not check for database existence"))]
    DbMeta { source: std::io::Error },

    #[snafu(display("Could not apply schema: {}", source))]
    DbSchema { source: sqlx::Error },
}

pub async fn open(db_path: &str) -> Result<sqlx::SqlitePool, OpenError> {
    let run_create = match fs::metadata(&db_path).await {
        Ok(_meta) => false,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => return Err(OpenError::DbMeta { source }),
    };
    if let Some(parent) = Path::new(db_path).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let pool = sqlx::Pool::builder()
        .max_size(2_u32)
        .build(&format!("sqlite://{}", db_path))
        .await
        .context(OpenDb)?;

    if run_create {
        let mut cxn = pool.acquire().await.unwrap();
        let schema = include_str!("../sql/schema.sql");
        sqlx::query(schema)
            .execute(&mut cxn)
            .await
            .context(DbSchema)?;
    }
    Ok(pool)
}

pub type Error = sqlx::Error;

#[derive(Copy, Clone, Ord, Eq, PartialEq, PartialOrd, Debug, sqlx::Type)]
#[sqlx(transparent)]
pub struct PathId(i32);

#[derive(Copy, Clone, sqlx::Type)]
#[sqlx(transparent)]
struct TagId(i32);

#[derive(Copy, Clone, Debug, sqlx::Type, PartialEq, Eq)]
#[sqlx(transparent)]
pub struct RootId(i32);

#[derive(Debug, Clone)]
pub struct RootData {
    path: String,
    id: RootId,
}

impl RootData {
    pub fn path(&self) -> &Path {
        Path::new(&self.path)
    }

    pub fn id(&self) -> RootId {
        self.id
    }

    pub fn root(&self, path: &RelativePath) -> String {
        // concating two strings so should never panic
        self.path()
            .join(path.as_ref())
            .into_os_string()
            .into_string()
            .unwrap()
    }
}

pub async fn update_timestamp(cxn: &mut SqliteConnection, data: &PathData) -> Result<(), Error> {
    sqlx::query!(
        "
        UPDATE relative_path
        SET unix_mtime = ?,
            unix_btime = ?
        WHERE file_path = ? AND
              root_id = ?
        ",
        data.time.mtime,
        data.time.btime,
        data.path.as_ref(),
        data.root_id
    )
    .execute(cxn)
    .await
    .map(|_| ())
}

pub async fn fetch_path_time(
    cxn: &mut SqliteConnection,
    root_id: RootId,
    path: &RelativePath,
) -> Result<Option<Time>, Error> {
    sqlx::query(
        "SELECT unix_mtime, unix_btime FROM relative_path WHERE file_path = ? AND root_id = ?",
    )
    .bind(path.as_ref())
    .bind(root_id)
    .try_map(|row: sqlx::sqlite::SqliteRow| {
        let btime: Option<UnixTimestamp> = row.get("unix_btime");
        Ok(Time {
            btime,
            mtime: row.get("unix_mtime"),
        })
    })
    .fetch_optional(cxn)
    .await
}

pub async fn insert_new_path(
    cxn: &mut SqliteConnection,
    path: &PathData,
    tags: &[Tag],
) -> Result<(), Error> {
    let path_id = insert_relative_path(cxn, path).await?;
    let mut tag_ids = Vec::with_capacity(tags.len());
    for tag in tags {
        tag_ids.push(get_or_insert_tag(cxn, tag.as_ref()).await?);
    }

    associate_path_with_tags(cxn, path_id, &tag_ids).await?;

    Ok(())
}

async fn associate_path_with_tags(
    mut cxn: &mut SqliteConnection,
    path: PathId,
    tags: &[TagId],
) -> Result<(), Error> {
    for tag in tags {
        sqlx::query!(
            "INSERT INTO path_tag(relative_path_id, tag_id) VALUES (?, ?)",
            path,
            tag
        )
        .execute(&mut cxn)
        .await?;
    }
    Ok(())
}

async fn get_or_insert_tag(mut cxn: &mut SqliteConnection, tag: &str) -> Result<TagId, Error> {
    sqlx::query!("INSERT OR IGNORE INTO tag(name) VALUES(?)", tag)
        .execute(&mut cxn)
        .await?;
    sqlx::query!("SELECT id FROM tag WHERE name = ?", tag)
        .fetch_one(cxn)
        .await
        .map(|row| TagId(row.id.unwrap()))
}

async fn insert_relative_path(
    mut cxn: &mut SqliteConnection,
    relative_path: &PathData,
) -> Result<PathId, Error> {
    sqlx::query!(
        "
        INSERT INTO
            relative_path(root_id, file_path, unix_mtime, unix_btime)
        VALUES
            (?, ?, ?, ?)",
        relative_path.root_id,
        relative_path.path.as_ref(),
        relative_path.time.mtime,
        relative_path.time.btime,
    )
    .execute(&mut cxn)
    .await?;

    sqlx::query!(
        "SELECT id FROM relative_path WHERE file_path = ?",
        relative_path.path.as_ref()
    )
    .fetch_one(cxn)
    .await
    .map(|row| PathId(row.id.unwrap()))
}

async fn fetch_tag_id(cxn: &mut SqliteConnection, tag: &str) -> Result<Option<i32>, Error> {
    sqlx::query!("SELECT id FROM tag WHERE name = ?", tag)
        .fetch_optional(cxn)
        .await
        .map(|row| row.map(|row| row.id.unwrap()))
}

async fn build_tag_where_clause(
    cxn: &mut SqliteConnection,
    tags: &[String],
) -> Result<String, Error> {
    let mut tag_ids = Vec::new();
    for tag in tags {
        if let Some(id) = fetch_tag_id(cxn, tag).await? {
            tag_ids.push(format!("{}", id));
        }
    }

    Ok(if tag_ids.is_empty() {
        String::new()
    } else {
        let id_list = tag_ids.join(",");
        format!("AND tag.id IN ({})", id_list)
    })
}

pub async fn get_or_insert_root(
    mut cxn: &mut SqliteConnection,
    path: String,
) -> Result<RootData, Error> {
    sqlx::query!("INSERT OR IGNORE INTO root(root_path) VALUES(?)", path)
        .execute(&mut cxn)
        .await?;

    sqlx::query!("SELECT id FROM root WHERE root_path = ?", &path)
        .fetch_one(&mut *cxn)
        .await
        .map(|row| RootData {
            id: RootId(row.id.unwrap()),
            path,
        })
}

pub async fn pickwp(
    cxn: &mut SqliteConnection,
    root_id: RootId,
    filter: &Filter,
) -> Result<Option<(PathId, RelativePath)>, Error> {
    let to_time = filter
        .to_time
        .map(UnixTimestamp::from)
        .unwrap_or(UnixTimestamp::from(std::i64::MAX));
    let from_time = filter
        .from_time
        .map(UnixTimestamp::from)
        .unwrap_or(UnixTimestamp::from(std::i64::MIN));

    let query = format!(
        "
            SELECT relative_path.id, relative_path.file_path
            FROM relative_path
            INNER JOIN path_tag ON path_tag.relative_path_id = relative_path.id
            INNER JOIN tag ON path_tag.tag_id = tag.id
            WHERE
                root_id = ?
                AND relative_path.unix_mtime <= ?
                AND relative_path.unix_mtime >= ?
                {}
            ORDER BY RANDOM()
            LIMIT 1
        ",
        build_tag_where_clause(cxn, &filter.tags).await?
    );

    sqlx::query(&query)
        .bind(root_id)
        .bind(to_time)
        .bind(from_time)
        .try_map(|row: SqliteRow| {
            let path: String = row.get("file_path");
            Ok((
                PathId(row.get::<i32, _>("id")),
                RelativePath::try_from(path).unwrap(),
            ))
        })
        .fetch_optional(cxn)
        .await
}
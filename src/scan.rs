use crate::{
    cache::{self, Cache},
    storage::{RelativePath, Time},
};
use slog_scope::info;
use snafu::ResultExt;
use std::{
    convert::TryFrom,
    future::Future,
    os::unix::prelude::*,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
        Mutex as StdMutex,
    },
};
use tgcd::Blake2bHash;
use tokio::{sync::mpsc, task};
use walkdir::{DirEntry, WalkDir};

struct CtxInner {
    cache: StdMutex<Cache>,
    is_running: AtomicBool,
}

pub struct ScanCtx {
    // this thing will only get locked in a blocking thread so just use the normal std mutex
    inner: Arc<CtxInner>,
    tx: mpsc::Sender<Vec<(RelativePath, Time)>>,
}

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    #[snafu(display("{}", source))]
    CacheOpen { source: cache::Error },

    #[snafu(display("{}", source))]
    #[snafu(context(false))]
    CacheErr { source: cache::Error },

    #[snafu(display("{}", source))]
    #[snafu(context(false))]
    TgcdConnect { source: tgcd::Error },
}

async fn scan_dir(
    ctx: Arc<CtxInner>,
    dir: String,
) -> Result<Vec<(RelativePath, Time, Option<Blake2bHash>)>, Error> {
    task::spawn_blocking(
        move || -> Result<Vec<(RelativePath, Time, Option<Blake2bHash>)>, Error> {
            let cache = ctx.cache.lock().unwrap();
            let mut ret = Vec::new();
            for entry in WalkDir::new(&dir) {
                if let Ok(entry) = entry {
                    let is_image = is_image(&entry);
                    if is_image {
                        let time = entry.metadata().map(|meta| Time::from_meta(&meta));
                        let relative = {
                            let unprefixed = entry.path().strip_prefix(&dir).unwrap();
                            RelativePath::try_from(unprefixed.to_owned())
                        };

                        if let (Ok(relative), Ok(time)) = (relative, time) {
                            let hash = if cache.path_exists(&relative).unwrap() {
                                None
                            } else {
                                // FIXME: unwrap
                                let ret = Blake2bHash::from_file(entry.path()).unwrap();
                                info!("Hashed"; slog::o!("path" => relative.as_ref(), "hash" => ret.to_string()));
                                Some(ret)
                            };
                            ret.push((relative, time, hash));
                        }
                    }
                }
            }
            Ok(ret)
        },
    )
    .await
    .unwrap()
}

struct RunGuard<'a>(&'a AtomicBool);

impl<'a> RunGuard<'a> {
    pub fn new(a: &'a AtomicBool) -> Self {
        a.store(true, Ordering::SeqCst);
        Self(a)
    }
}

impl<'a> Drop for RunGuard<'a> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl ScanCtx {
    pub fn new() -> Result<(Self, mpsc::Receiver<Vec<(RelativePath, Time)>>), Error> {
        let (tx, rx) = mpsc::channel(1);
        let inner = Arc::new(CtxInner {
            cache: StdMutex::new(Cache::open().context(CacheOpen)?),
            is_running: AtomicBool::new(false),
        });

        Ok((Self { tx, inner }, rx))
    }

    pub fn scan(&self, dir: String) -> Option<impl Future<Output = Result<(), Error>>> {
        if self.inner.is_running.load(Ordering::SeqCst) {
            None
        } else {
            // TODO: use the "threaded" scheduler which doesn't actually mean multithreaded
            // but work stealing
            let this = self.inner.clone();
            let mut tx = self.tx.clone();
            Some(async move {
                let _ = RunGuard::new(&this.is_running);
                let mut tgcd = tgcd::TgcdClient::from_global_config().await?;

                let scan_res = scan_dir(this.clone(), dir).await?;

                let this = this.clone();
                let (txn_tx, txn_rx) =
                    crossbeam_channel::unbounded::<(RelativePath, Vec<tgcd::Tag>)>();
                let txn_task = task::spawn_blocking(move || {
                    let mut cache = this.cache.lock().unwrap();
                    let txn = cache.transaction().unwrap();
                    for (relative, tags) in txn_rx {
                        info!("Cached"; slog::o!("path" => relative.as_ref()));
                        txn.insert_path_with_tags(&relative, &tags).unwrap();
                    }
                    txn.commit().map_err(Error::from)
                });

                let mut ret = Vec::with_capacity(scan_res.len());
                for (relative, time, hash) in scan_res {
                    if let Some(hash) = hash {
                        let tags = tgcd.get_tags(&hash).await?;
                        txn_tx.send((relative.clone(), tags)).unwrap();
                    }
                    ret.push((relative, time));
                }
                drop(txn_tx);

                txn_task.await.unwrap()?;

                tx.send(ret).await.unwrap();

                Ok(())
            })
        }
    }

    pub fn get_cache(&self) -> std::sync::MutexGuard<Cache> {
        self.inner.cache.lock().unwrap()
    }
}

fn is_image(ent: &DirEntry) -> bool {
    static IMAGE_EXTENSIONS: phf::Set<&'static [u8]> = phf::phf_set! {
        b"jpe",
        b"jpeg",
        b"jpg",
        b"png",
    };

    ent.file_type().is_file()
        && ent
            .path()
            .extension()
            .map(|ext| IMAGE_EXTENSIONS.contains(ext.as_bytes()))
            .unwrap_or(false)
}

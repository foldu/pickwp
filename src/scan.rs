use crate::{
    data::{PathData, RelativePath, Time},
    db,
};
use futures_util::stream::{Stream, StreamExt};
use sqlx::SqlitePool;
use std::{convert::TryFrom, path::PathBuf, sync::Arc};
use tokio::{
    sync::{
        mpsc::{self, error::TrySendError},
        Mutex,
    },
    task,
};

pub struct ImageScanner {
    // uses tokio mutex instead of std mutex even if not using async capabilities because
    // std uses the suboptimal POSIX mutexen
    scanning: Arc<Mutex<()>>,
}

fn scan(root: PathBuf) -> tokio::sync::mpsc::Receiver<(PathBuf, PathData)> {
    let (mut tx, rx) = mpsc::channel(1);
    task::spawn_blocking(move || {
        for ent in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|ent| ent.ok())
            .filter(|ent| ent.file_type().is_file())
        {
            if let Ok(stat) = ent.metadata() {
                let absolute = ent.into_path();
                let relative =
                    RelativePath::try_from(absolute.strip_prefix(&root).unwrap().to_owned())
                        .unwrap();
                let data = PathData {
                    path: relative,
                    time: Time {
                        mtime: stat.modified().unwrap().into(),
                        btime: stat.created().ok().map(|time| time.into()),
                    },
                };
                let mut to_send = (absolute, data);

                loop {
                    match tx.try_send(to_send) {
                        Ok(_) => break,
                        Err(TrySendError::Full(a)) => {
                            to_send = a;
                            slog_scope::debug!("Scan channel buffer full");
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(TrySendError::Closed(_)) => return,
                    }
                }
            }
        }
    });

    rx
}

struct CpuJobSet<T> {
    tx: Option<tokio::sync::mpsc::Sender<T>>,
}

impl<T> CpuJobSet<T>
where
    T: Send + 'static,
{
    fn buffered(bufsiz: usize) -> (Self, impl Stream<Item = T>) {
        let (tx, rx) = tokio::sync::mpsc::channel(bufsiz);
        (Self { tx: Some(tx) }, rx)
    }

    fn execute(&self, f: impl FnOnce() -> T + Send + 'static) {
        let mut tx = self.tx.as_ref().unwrap().clone();
        rayon::spawn(move || {
            let mut ret = f();
            loop {
                match tx.try_send(ret) {
                    Ok(_) => break,
                    Err(TrySendError::Full(a)) => {
                        ret = a;
                        slog_scope::debug!("Jobset buffer full");
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(TrySendError::Closed(_)) => return,
                }
            }
        })
    }

    fn stop(&mut self) {
        self.tx = None;
    }
}

impl ImageScanner {
    pub fn new() -> Self {
        Self {
            scanning: Default::default(),
        }
    }

    pub fn start_scan(&mut self, pool: &SqlitePool, root: PathBuf) {
        let pool = pool.clone();
        let scanning = self.scanning.clone();
        let task = task::spawn(async move {
            if let Ok(_) = scanning.try_lock() {
                slog_scope::debug!("Starting scan");
                let mut scan = scan(root);
                let (mut spawner, mut hash_jobs) = CpuJobSet::buffered(32);
                let mut tgcd = tgcd::TgcdClient::from_global_config().await.unwrap();

                let mut txn = pool.begin().await.unwrap();

                let mut scan_done = false;
                loop {
                    tokio::select! {
                        next = scan.next(), if !scan_done => {
                            match next {
                                Some((absolute, path_data)) => {
                                    match db::fetch_path_time(&mut txn, &path_data.path).await? {
                                        Some(time) if time == path_data.time => (),
                                        Some(time) => {
                                            slog_scope::info!("Updating meta of {}", path_data.path.as_ref());
                                            db::update_timestamp(&mut txn, &PathData { time, ..path_data }).await?;

                                        }
                                        None => {
                                            spawner.execute(move || -> Result<_, std::io::Error> {
                                                let hash = tgcd::Blake2bHash::from_file(absolute)?;
                                                Ok((path_data, hash))
                                            });
                                        }
                                    }
                                }
                                None => {
                                    scan_done = true;
                                    spawner.stop();
                                }
                            }
                        }
                        Some(hashed) = hash_jobs.next() => {
                            if let Ok((path_data, hash)) = hashed {
                                let tags = tgcd.get_tags(&hash).await.unwrap();
                                slog_scope::info!("Found new file: {}", path_data.path.as_ref());
                                db::insert_new_path(&mut txn, &path_data, &tags).await?;
                            }
                        }
                        else => break,
                    }
                }

                txn.commit().await?;
            };
            Ok(())
        });

        task::spawn(async move {
            if let Err(e) = task.await.unwrap() {
                let e: anyhow::Error = e;
                slog_scope::error!("{}", e);
            }
        });
    }
}

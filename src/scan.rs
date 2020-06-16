use crate::{
    data::{PathData, RelativePath, Time},
    db::{self, RootData, RootId},
};
use futures_util::stream::{Stream, StreamExt};
use sqlx::SqlitePool;
use std::{convert::TryFrom, path::PathBuf, sync::Arc, time::Instant};
use tokio::{
    sync::{
        mpsc::{self, error::TrySendError},
        oneshot,
        Mutex,
    },
    task,
};

#[derive(derive_more::Deref, Clone)]
pub struct ImageScanner(Arc<ScanInner>);

#[doc(hidden)]
pub struct ScanInner {
    scanning: Mutex<()>,
    state: Mutex<ScanState>,
}

enum ScanState {
    Scanning {
        root: RootData,
        abort_handle: Option<oneshot::Sender<()>>,
    },
    Idle,
}

impl ScanState {
    fn scanning(root: RootData) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (
            Self::Scanning {
                root,
                abort_handle: Some(tx),
            },
            rx,
        )
    }
}

fn scan(root: RootData) -> tokio::sync::mpsc::Receiver<(PathBuf, PathData)> {
    let (mut tx, rx) = mpsc::channel(1);
    task::spawn_blocking(move || {
        for ent in walkdir::WalkDir::new(root.path())
            .into_iter()
            .filter_map(|ent| ent.ok())
            .filter(|ent| ent.file_type().is_file())
        {
            if let Ok(stat) = ent.metadata() {
                let absolute = ent.into_path();
                let relative =
                    RelativePath::try_from(absolute.strip_prefix(root.path()).unwrap().to_owned())
                        .unwrap();
                let data = PathData {
                    root_id: root.id(),
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
                            //tracing::debug!("Scan channel buffer full");
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
                        tracing::debug!("Jobset buffer full");
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
        Self(Arc::new(ScanInner {
            scanning: Default::default(),
            state: Mutex::new(ScanState::Idle),
        }))
    }

    pub fn start_scan(&mut self, pool: &SqlitePool, root: RootData) {
        let pool = pool.clone();
        let this = self.0.clone();
        let task = task::spawn(async move {
            if let Ok(_) = this.scanning.try_lock() {
                let scan_begin = Instant::now();

                tracing::info!("Starting scan");

                let (state, mut abort) = ScanState::scanning(root.clone());
                {
                    *this.state.lock().await = state;
                }

                let root_id = root.id();
                let mut scan = scan(root);
                let (mut spawner, mut hash_jobs) = CpuJobSet::buffered(32);
                // FIXME: get this thing from function args
                let mut tgcd = tgcd::TgcdClient::from_global_config().await.unwrap();

                let mut txn = pool.begin().await.unwrap();

                let mut scan_done = false;
                let mut loop_done = false;
                loop {
                    tokio::select! {
                        _ = &mut abort, if !loop_done => {
                            // don't commit txn
                            return Ok(());
                        }
                        next = scan.next(), if !scan_done => {
                            match next {
                                Some((absolute, path_data)) => {
                                    match db::fetch_path_time(&mut txn, root_id, &path_data.path).await? {
                                        Some(time) if time == path_data.time => (),
                                        Some(time) => {
                                            tracing::info!("Updating meta of {}", path_data.path.as_ref());
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
                        job = hash_jobs.next(), if !loop_done => {
                            match job {
                                Some(Ok((path_data, hash))) => {
                                    let tags = tgcd.get_tags(&hash).await.unwrap();
                                    tracing::info!("Found new file: {}", path_data.path.as_ref());
                                    db::insert_new_path(&mut txn, &path_data, &tags).await?;
                                }
                                None => {
                                    loop_done = true;
                                }
                                _ => (),
                            }
                        }
                        else => break,
                    }
                }

                tracing::info!(
                    duration = %humantime::Duration::from(Instant::now().duration_since(scan_begin)),
                    "Finished scan",
                );

                txn.commit().await?;
            };

            Ok(())
        });

        let this = self.0.clone();
        task::spawn(async move {
            if let Err(e) = task.await.unwrap() {
                let e: anyhow::Error = e;
                tracing::error!("{}", e);
            }
            *this.state.lock().await = ScanState::Idle;
        });
    }

    pub async fn abort_if_root_differs(&mut self, root_id: RootId) {
        let mut state = self.0.state.lock().await;
        match *state {
            ScanState::Scanning {
                ref mut abort_handle,
                ref root,
            } => {
                if root.id() == root_id {
                    let _ = abort_handle.take().and_then(|handle| handle.send(()).ok());
                };
            }
            _ => (),
        }
    }
}

use futures_util::stream::{Stream, StreamExt};
use inotify::{EventMask, Inotify, WatchMask};
use std::{future::Future, path::Path, time::Duration};
use tokio::sync::mpsc;

enum WatchResult {
    RxDropped,
    ParentDeleted,
}

async fn watch(
    inotify: &mut Inotify,
    buf: &mut Vec<u8>,
    tx: &mut mpsc::Sender<Vec<u8>>,
    path: &Path,
) -> Result<WatchResult, std::io::Error> {
    let file_name = path.file_name().unwrap();
    let mut events = inotify.event_stream(buf)?;
    while let Some(ev) = events.next().await {
        let ev = ev.unwrap();
        if ev.mask.contains(EventMask::DELETE_SELF) {
            return Ok(WatchResult::ParentDeleted);
        } else {
            match ev.name {
                Some(name) if name == file_name => {
                    let cont = tokio::fs::read(path).await?;
                    if let Err(_) = tx.send(cont).await {
                        return Ok(WatchResult::RxDropped);
                    }
                }
                _ => (),
            }
        }
    }
    panic!("inotify stream stopped for some reason")
}

pub struct FileWatcher {
    retry_delay: Duration,
}

impl Default for FileWatcher {
    fn default() -> Self {
        Self {
            retry_delay: Duration::from_secs(5),
        }
    }
}

impl FileWatcher {
    pub fn watch(
        self,
        path: impl AsRef<Path>,
    ) -> Result<
        (
            impl Future<Output = Result<(), std::io::Error>>,
            impl Stream<Item = Vec<u8>>,
        ),
        std::io::Error,
    > {
        watch_file(path, self.retry_delay)
    }
}

fn watch_file(
    path: impl AsRef<Path>,
    retry_delay: Duration,
) -> Result<
    (
        impl Future<Output = Result<(), std::io::Error>>,
        impl Stream<Item = Vec<u8>>,
    ),
    std::io::Error,
> {
    let path = path.as_ref().to_owned();
    let (mut tx, rx) = mpsc::channel(1);
    let mut inotify = Inotify::init()?;
    let task = async move {
        let mut buf = vec![0; 4 * (1 << 10)];
        let parent = path.parent().unwrap();
        loop {
            tracing::info!("Starting watch loop");
            match inotify.add_watch(parent, WatchMask::CLOSE_WRITE | WatchMask::DELETE_SELF) {
                Ok(desc) => {
                    match watch(&mut inotify, &mut buf, &mut tx, &path).await {
                        Ok(WatchResult::ParentDeleted) | Err(_) => {}
                        Ok(WatchResult::RxDropped) => return Ok(()),
                    }
                    inotify.rm_watch(desc)?;
                }
                // maybe do something here
                Err(_) => (),
            }

            tokio::time::delay_for(retry_delay).await;
        }
    };
    Ok((task, rx))
}

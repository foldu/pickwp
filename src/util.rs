use std::time::Duration;

use futures_util::stream::{Stream, StreamExt};
use tokio::{sync::mpsc, task, time};

pub trait PathBufExt {
    fn into_string(self) -> Result<String, std::ffi::OsString>;
}

impl PathBufExt for std::path::PathBuf {
    fn into_string(self) -> Result<String, std::ffi::OsString> {
        self.into_os_string().into_string()
    }
}

#[derive(Clone, Debug)]
pub struct Preempter {
    tx: mpsc::Sender<()>,
}

impl Preempter {
    pub async fn preempt(&mut self) {
        self.tx.send(()).await.unwrap()
    }
}

pub fn preemptible_interval(timeout: Duration) -> (Preempter, impl Stream<Item = ()>) {
    let (preempt_tx, preempt_rx) = mpsc::channel(4);

    let (mut inner_tx, inner_rx) = mpsc::channel(4);
    task::spawn(async move {
        let mut interval = time::interval(timeout).fuse();
        let mut preempt_rx = preempt_rx.fuse();
        loop {
            futures_util::select! {
                _ = interval.next() => {
                }
                preemption = preempt_rx.next() => {
                    // preempter dropped, time to stop
                    if let None = preemption {
                        break;
                    }
                    interval = time::interval(timeout).fuse();
                }
            }

            let _ = inner_tx.send(()).await;
        }
    });

    (Preempter { tx: preempt_tx }, inner_rx)
}

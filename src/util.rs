use std::time::Duration;

use futures::prelude::*;
use tokio::{runtime::current_thread, sync::mpsc, timer::Interval};

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
    pub async fn preempt(&mut self) -> Result<(), tokio::sync::mpsc::error::SendError> {
        self.tx.send(()).await
    }
}

pub fn preemptible_interval(timeout: Duration) -> (Preempter, impl Stream<Item = ()>) {
    let (preempt_tx, preempt_rx) = mpsc::channel(4);

    let (mut inner_tx, inner_rx) = mpsc::channel(4);
    current_thread::spawn(async move {
        let mut interval = Interval::new_interval(timeout).fuse();
        let mut preempt_rx = preempt_rx.fuse();
        loop {
            futures::select! {
                _ = interval.next() => {
                }
                preemption = preempt_rx.next() => {
                    // preempter dropped, time to stop
                    if let None = preemption {
                        break;
                    }
                    interval = Interval::new_interval(timeout).fuse();
                }
            }

            let _ = inner_tx.send(()).await;
        }
    });

    (Preempter { tx: preempt_tx }, inner_rx)
}

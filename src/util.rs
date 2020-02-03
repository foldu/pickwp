use std::time::Duration;
use futures_util::{
    future,
    stream::{Stream, StreamExt},
};
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

pub fn preemptible_interval(time: Duration) -> (Preempter, impl Stream<Item = ()>) {
    let (preempt_tx, mut preempt_rx) = mpsc::channel(1);

    let (mut inner_tx, inner_rx) = mpsc::channel(1);
    task::spawn(async move {
        let mut timeout = time::delay_for(time);
        loop {
            future::select(timeout, preempt_rx.next()).await;
            timeout = time::delay_for(time);
            let _ = inner_tx.send(()).await;
        }
    });

    (Preempter { tx: preempt_tx }, inner_rx)
}

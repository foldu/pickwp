use directories::ProjectDirs;
use futures_util::{
    future,
    stream::{Stream, StreamExt},
};
use std::{path::PathBuf, time::Duration};
use tokio::{sync::mpsc, task, time};

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
            if let Err(_) = inner_tx.send(()).await {
                break;
            }
        }
    });

    (Preempter { tx: preempt_tx }, inner_rx)
}

pub struct AppPaths {
    // NOTE: must be String because sqlite needs a valid UTF-8 path
    pub db_file: String,
    pub rt_dir: PathBuf,
    pub config_file: PathBuf,
}

impl AppPaths {
    pub fn get() -> Option<Self> {
        let dirs = ProjectDirs::from("org", "foldu", env!("CARGO_PKG_NAME"))?;
        let db_file = dirs
            .data_dir()
            .join("db.sqlite")
            .into_os_string()
            .into_string()
            // FIXME: unwrap
            .unwrap();

        Some(AppPaths {
            db_file,
            rt_dir: dirs
                .runtime_dir()
                .map(|dir| dir.to_owned())
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp/pickwp")),
            config_file: dirs.config_dir().join("config.toml"),
        })
    }
}

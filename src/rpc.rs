mod client;
mod server;
mod tarpc_unix_transport;

pub use client::{connect, Error as ClientError};
pub use server::{bind, Error as ServerError};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[tarpc::service]
pub trait PickwpService {
    async fn refresh();
    async fn scan();
    async fn get_wallpapers() -> BTreeMap<String, Option<String>>;
    async fn toggle_freeze() -> bool;
}

struct RtPath {
    lockpath: PathBuf,
    sockpath: PathBuf,
}

impl RtPath {
    fn new(rtdir: impl AsRef<Path>) -> Self {
        let rtdir = rtdir.as_ref();
        Self {
            lockpath: rtdir.join("pickwp.lock"),
            sockpath: rtdir.join("pickwp.sock"),
        }
    }
}

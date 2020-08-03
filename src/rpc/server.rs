use crate::{
    daemon,
    rpc::PickwpService,
    unix::{mkdir, LockFile, LockFileError},
};
use futures_util::{future::Future, stream::StreamExt};
use nix::sys::stat::Mode;
use snafu::ResultExt;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tarpc::context::Context;
use tokio::net::UnixListener;

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    #[snafu(display(
        "Daemon is already running. Please kill the currently running instance and try again"
    ))]
    AlreadyRunning,

    #[snafu(display("Can't create runtime directory: {}", source))]
    CreateRtDir {
        rtdir: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("Error when trying to create lock file in {}: {}", lockpath.display(), source))]
    CreateLock {
        lockpath: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("Can't bind to socket: {}", source), context(false))]
    Bind { source: std::io::Error },
}

pub struct Listener {
    sock: UnixListener,
    lockfile: LockFile,
}

impl Listener {
    pub fn serve(self, state: daemon::State) -> impl Future<Output = ()> {
        use tarpc::rpc::server::Handler;
        // need to do this instead of
        // self.sock
        //    .incoming()
        //    .filter_map(|stream| futures_util::future::ready(stream.ok()))
        //    .map(|stream| {
        //        tarpc::serde_transport::Transport::from((
        //            stream,
        //            tokio_serde::formats::Cbor::default(),
        //        ))
        //    }),
        // because tokio::net::Incoming takes a reference to the listener and
        // borrowck doesn't like it
        let Self { lockfile, sock } = self;
        let incoming =
            super::tarpc_unix_transport::incoming(sock, tokio_serde::formats::Json::default);
        async move {
            let _lock = lockfile;
            tarpc::server::new(Default::default())
                .incoming(incoming.filter_map(|stream| async { stream.ok() }))
                .respond_with(state.serve())
                .await
        }
    }
}

#[tarpc::server]
impl super::PickwpService for daemon::State {
    async fn refresh(self, _: Context) {
        if let Some(state) = self.lock().await.as_mut() {
            state.refresh_preempt.preempt().await;
        }
    }

    async fn scan(self, _: Context) {
        if let Some(state) = self.lock().await.as_mut() {
            state.scan_preempt.preempt().await;
        }
    }

    async fn get_wallpapers(self, _: Context) -> BTreeMap<String, Option<String>> {
        if let Some(state) = self.lock().await.as_ref() {
            state.current_wps.clone()
        } else {
            Default::default()
        }
    }

    async fn toggle_freeze(self, _: Context) -> bool {
        if let Some(state) = self.lock().await.as_mut() {
            state.frozen = !state.frozen;
            state.frozen
        } else {
            false
        }
    }
}

pub fn bind(rtdir: impl AsRef<Path>) -> Result<Listener, Error> {
    let rtdir = rtdir.as_ref();
    let rtpath = super::RtPath::new(&rtdir);
    // I don't care about the permissions here
    if let Some(parent) = rtdir.parent() {
        let _ = std::fs::create_dir_all(&parent);
    }

    match mkdir(rtdir, Mode::from_bits_truncate(0o700)) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        ret => ret.with_context(|| CreateRtDir {
            rtdir: rtdir.clone(),
        })?,
    }

    let lockfile = LockFile::lock(&rtpath.lockpath).map_err(|e| match e {
        LockFileError::Locked => Error::AlreadyRunning,
        LockFileError::Create { source } => Error::CreateLock {
            lockpath: rtpath.lockpath.clone(),
            source,
        },
    })?;

    let _ = std::fs::remove_file(&rtpath.sockpath);
    let sock = UnixListener::bind(&rtpath.sockpath)?;
    Ok(Listener { sock, lockfile })
}

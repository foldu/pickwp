use crate::unix::{LockFile, LockFileError};
use snafu::ResultExt;
use std::path::Path;
use tokio::net::UnixStream;

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    #[snafu(display("pickwp is not running"))]
    NotRunning,

    #[snafu(display("Could not connect to rpc sock: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("Could not connect to service: {}", source))]
    Spawn { source: std::io::Error },
}

pub async fn connect(rtdir: impl AsRef<Path>) -> Result<super::PickwpServiceClient, Error> {
    let rtpath = super::RtPath::new(rtdir);
    match LockFile::lock(&rtpath.lockpath) {
        Err(LockFileError::Locked) => (),
        _ => {
            return Err(Error::NotRunning);
        }
    };
    // race condition lmao
    let stream = UnixStream::connect(&rtpath.sockpath)
        .await
        .context(Connect)?;

    let transport = super::tarpc_unix_transport::new(stream, tokio_serde::formats::Json::default());

    super::PickwpServiceClient::new(tarpc::client::Config::default(), transport)
        .spawn()
        .context(Spawn)
}

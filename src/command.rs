use bytes::Bytes;
use futures::{future, prelude::*};
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use tokio::{
    codec::{Framed, LengthDelimitedCodec},
    net::unix::{UnixListener, UnixStream},
    prelude::*,
};

const SOCK_PATH: &'static str = "\0pickwp";

pub struct Request {
    pub cmd: Command,
    framed: Framed<UnixStream, LengthDelimitedCodec>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Reply {
    Ok,
}

impl Request {
    pub async fn reply(mut self, reply: Reply) -> Result<(), IpcError> {
        let ret = Bytes::from(bincode::serialize(&reply).context(Bincode)?);
        self.framed.send(ret).await.context(SendReply)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Command {
    Refresh,
}

#[derive(Snafu, Debug)]
pub enum IpcError {
    #[snafu(display("Can't bind to {}: {}", path, source))]
    Bind {
        path: &'static str,
        source: std::io::Error,
    },

    #[snafu(display("Can't connect to {}: {}", path, source))]
    Connect {
        path: &'static str,
        source: std::io::Error,
    },

    #[snafu(display("Connection hung up before sending cmd"))]
    Hup,

    #[snafu(display("Connection hung up"))]
    IntermittentIo { source: std::io::Error },

    #[snafu(display("Reply"))]
    SendReply { source: std::io::Error },

    #[snafu(display("Can't bincode"))]
    Bincode { source: bincode::Error },
}

async fn read_cmd(conn: UnixStream) -> Result<Request, IpcError> {
    let mut framed = Framed::new(conn, LengthDelimitedCodec::new());
    let buf = framed
        .next()
        .await
        .ok_or(IpcError::Hup)?
        .context(IntermittentIo)?;

    let cmd = bincode::deserialize(&buf).context(Bincode)?;

    Ok(Request { cmd, framed })
}

pub fn command_stream() -> Result<impl Stream<Item = Request>, IpcError> {
    let listener = UnixListener::bind(SOCK_PATH).context(Bind { path: SOCK_PATH })?;

    Ok(listener
        .incoming()
        .filter_map(|conn| future::ready(conn.ok()))
        .map(read_cmd)
        .filter_map(|cmd| async { cmd.await.ok() }))
}

pub async fn send_command(cmd: Command) -> Result<Reply, IpcError> {
    let stream = UnixStream::connect(SOCK_PATH)
        .await
        .context(Connect { path: SOCK_PATH })?;
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

    let buf = bincode::serialize(&cmd).context(Bincode)?;
    framed
        .send(Bytes::from(buf))
        .await
        .context(IntermittentIo)?;

    let buf = framed
        .next()
        .await
        .ok_or(IpcError::Hup)?
        .context(IntermittentIo)?;

    bincode::deserialize(&buf).context(Bincode)
}

use futures_util::stream::Stream;
use serde::Deserialize;
use tokio::stream::StreamExt;
use tokio_i3ipc::{
    event::{Event, Subscribe, WorkspaceChange},
    I3,
};

#[derive(Deserialize, Copy, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Fill,
    Tile,
}

#[async_trait::async_trait]
pub trait Monitor {
    async fn idents(&mut self) -> Result<Vec<String>, Error>;
    async fn set_wallpaper(&mut self, mode: Mode, ident: &str, path: &str) -> Result<(), Error>;
    async fn display_changed(
        &self,
    ) -> Result<Box<dyn Stream<Item = Result<(), Error>> + Unpin>, Error>;
}

pub struct Sway(I3);

impl Sway {
    pub async fn new() -> Result<Self, Error> {
        I3::connect().await.map_err(Error::new).map(Self)
    }
}

#[async_trait::async_trait]
impl Monitor for Sway {
    async fn idents(&mut self) -> Result<Vec<String>, Error> {
        self.0
            .get_outputs()
            .await
            .map_err(Error::new)
            .map(|out| out.into_iter().map(|out| out.name).collect())
    }

    async fn set_wallpaper(&mut self, mode: Mode, ident: &str, path: &str) -> Result<(), Error> {
        let mode = match mode {
            Mode::Fill => "fill",
            Mode::Tile => "tile",
        };

        let escaped_path = path.replace('"', "\"");
        let cmd = format!(r#"output {} background "{}" {}"#, ident, escaped_path, mode);

        self.0
            .run_command(&cmd)
            .await
            .map_err(Error::new)
            .and_then(|ret| {
                let ret = &ret[0];
                if ret.success {
                    Ok(())
                } else {
                    Err(Error::new(match &ret.error {
                        Some(e) => format!("Can't set wallpaper: {}", e),
                        None => format!("Can't set wallpaper"),
                    }))
                }
            })
    }

    async fn display_changed(
        &self,
    ) -> Result<Box<dyn Stream<Item = Result<(), Error>> + Unpin>, Error> {
        let mut cxn = I3::connect().await.map_err(Error::new)?;
        cxn.subscribe(&[Subscribe::Workspace])
            .await
            .map_err(Error::new)?;
        Ok(Box::new(cxn.listen().filter_map(|evt| match evt {
            Ok(Event::Workspace(evt)) if evt.change == WorkspaceChange::Reload => Some(Ok(())),
            _ => None,
        })))
    }
}

#[derive(Debug)]
pub struct Error(Box<dyn std::error::Error + Sync + Send>);

impl Error {
    fn new(e: impl Into<Box<dyn std::error::Error + Sync + Send>>) -> Self {
        Self(e.into())
    }
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

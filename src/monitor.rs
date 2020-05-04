use serde::Deserialize;
use tokio_i3ipc::I3;

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
}

#[derive(Default)]
pub struct Sway(Option<I3>);

impl Sway {
    async fn get_cxn(&mut self) -> Result<&mut I3, Error> {
        match self.0 {
            Some(ref mut cxn) => Ok(cxn),
            None => {
                let cxn = I3::connect().await.map_err(Error::new)?;
                self.0 = Some(cxn);
                Ok(self.0.as_mut().unwrap())
            }
        }
    }
}

macro_rules! cut_cxn_on_err {
    ($this:expr, $ret:expr) => {
        match $ret {
            Ok(ret) => Ok(ret),
            Err(e) => {
                $this.0 = None;
                Err(e)
            }
        }
    };
}

#[async_trait::async_trait]
impl Monitor for Sway {
    async fn idents(&mut self) -> Result<Vec<String>, Error> {
        let cxn = self.get_cxn().await?;
        cut_cxn_on_err!(
            self,
            cxn.get_outputs()
                .await
                .map_err(Error::new)
                .map(|out| out.into_iter().map(|out| out.name).collect())
        )
    }

    async fn set_wallpaper(&mut self, mode: Mode, ident: &str, path: &str) -> Result<(), Error> {
        let cxn = self.get_cxn().await?;
        let mode = match mode {
            Mode::Fill => "fill",
            Mode::Tile => "tile",
        };

        // FIXME: escaping
        let cmd = format!(r#"output {} background "{}" {}"#, ident, path, mode);

        cut_cxn_on_err!(
            self,
            cxn.run_command(&cmd)
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
        )
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

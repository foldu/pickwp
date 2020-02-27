use crate::config;
use serde::Deserialize;
use snafu::{ Snafu};
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

impl From<config::Backend> for Box<dyn Monitor> {
    fn from(other: config::Backend) -> Self {
        match other {
            config::Backend::Sway => Box::new(Sway::default()),
        }
    }
}

#[derive(Default)]
pub struct Sway(Option<I3>);

impl Sway {
    async fn get_cxn(&mut self) -> Result<&mut I3, Error> {
        match self.0 {
            Some(ref mut cxn) => Ok(cxn),
            None => {
                let cxn = I3::connect().await.map_err(|source| Error::Generic {
                    source: source.into(),
                })?;
                self.0.replace(cxn);
                Ok(self.0.as_mut().unwrap())
            }
        }
    }
}

macro_rules! cut_cxn_if {
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
        cut_cxn_if!(
            self,
            cxn.get_outputs()
                .await
                .map_err(|e| Error::Generic { source: e.into() })
                .map(|out| out.into_iter().map(|out| out.name).collect())
        )
    }

    async fn set_wallpaper(&mut self, mode: Mode, ident: &str, path: &str) -> Result<(), Error> {
        let cxn = self.get_cxn().await?;
        let mode = match mode {
            Mode::Fill => "fill",
            Mode::Tile => "tile",
        };

        let cmd = format!(r#"output {} background "{}" {}"#, ident, path, mode);

        cut_cxn_if!(
            self,
            cxn.run_command(&cmd)
                .await
                .map_err(|e| Error::Generic { source: e.into() })
                .and_then(|ret| {
                    let ret = &ret[0];
                    if ret.success {
                        Ok(())
                    } else {
                        Err(Error::Generic {
                            source: match &ret.error {
                                Some(e) => format!("Can't set wallpaper: {}", e).into(),
                                None => format!("Can't set wallpaper").into(),
                            },
                        })
                    }
                })
        )
    }
}

#[derive(Snafu, Debug)]
pub enum Error {
    #[snafu(display("{}", source))]
    Generic { source: Box<dyn std::error::Error> },

    #[snafu(display("Can't get screens: {}", source))]
    GetScreens { source: Box<dyn std::error::Error> },

    #[snafu(display("Can't set wallpaper {} for {}: {}", path, ident, source))]
    SetWallpaper {
        source: Box<dyn std::error::Error>,
        path: String,
        ident: String,
    },
}

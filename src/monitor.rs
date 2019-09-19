use std::process::Command;

use serde::Deserialize;
use snafu::Snafu;

use crate::config;

#[derive(Deserialize, Copy, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Fill,

    Tile,
}

pub trait Monitor {
    fn idents(&mut self) -> Result<Vec<String>, Error>;
    fn set_wallpaper(&mut self, mode: Mode, ident: &str, path: &str) -> Result<(), Error>;
}

impl From<config::Backend> for Box<dyn Monitor> {
    fn from(other: config::Backend) -> Self {
        match other {
            config::Backend::Sway => Box::new(Sway),
        }
    }
}

pub struct Sway;

#[derive(Deserialize)]
struct GetOutputField {
    name: String,
}

impl Monitor for Sway {
    fn idents(&mut self) -> Result<Vec<String>, Error> {
        let output = Command::new("swaymsg")
            .arg("-rt")
            .arg("get_outputs")
            .output()
            .map_err(|e| Error::GetScreens {
                source: format!("Can't launch swaymsg: {}", e).into(),
            })?;

        let parsed: Vec<GetOutputField> =
            serde_json::from_slice(&output.stdout).map_err(|e| Error::GetScreens {
                source: Box::new(e),
            })?;

        Ok(parsed.into_iter().map(|field| field.name).collect())
    }

    fn set_wallpaper(&mut self, mode: Mode, ident: &str, path: &str) -> Result<(), Error> {
        let mode = match mode {
            Mode::Fill => "fill",
            Mode::Tile => "tile",
        };

        let arg = format!(r#"output {} background "{}" {}"#, ident, path, mode);

        Command::new("swaymsg")
            .arg(arg)
            .output()
            .map_err(|e| Error::SetWallpaper {
                path: path.to_string(),
                ident: ident.to_string(),
                source: format!("Error launching swaymsg: {}", e).into(),
            })?;

        Ok(())
    }
}

#[derive(Snafu, Debug)]
pub enum Error {
    #[snafu(display("Can't get screens: {}", source))]
    GetScreens { source: Box<dyn std::error::Error> },

    #[snafu(display("Can't set wallpaper {} for {}: {}", path, ident, source))]
    SetWallpaper {
        source: Box<dyn std::error::Error>,
        path: String,
        ident: String,
    },
}

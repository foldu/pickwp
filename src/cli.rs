use structopt::StructOpt;

#[derive(StructOpt, Debug)]
pub struct Opt {
    #[structopt(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(StructOpt, Debug)]
pub enum Cmd {
    /// Select some new wallpapers
    Refresh,

    /// Rescan wallpaper directory
    Rescan,

    /// Print currently selected wallpapers
    Current,

    /// Stop changing current wallpapers
    ToggleFreeze,
}

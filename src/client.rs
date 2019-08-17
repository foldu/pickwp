use structopt::StructOpt;

use crate::command::{send_command, Command, IpcError};

// duplicate of command::Command
#[derive(StructOpt, Debug)]
pub enum Subcmd {
    #[structopt(name = "refresh")]
    Refresh,

    #[structopt(name = "rescan")]
    Rescan,
}

pub async fn run(cmd: Subcmd) -> Result<(), IpcError> {
    match cmd {
        Subcmd::Refresh => {
            send_command(Command::Refresh).await?;
        }
        Subcmd::Rescan => {
            send_command(Command::Rescan).await?;
        }
    }

    Ok(())
}

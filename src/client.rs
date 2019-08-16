use structopt::StructOpt;

use crate::command::{send_command, Command, IpcError};

#[derive(StructOpt, Debug)]
pub enum Subcmd {
    #[structopt(name = "refresh")]
    Refresh,
}

pub async fn run(cmd: Subcmd) -> Result<(), IpcError> {
    match cmd {
        Subcmd::Refresh => {
            send_command(Command::Refresh).await?;
        }
    }

    Ok(())
}

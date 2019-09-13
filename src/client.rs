use oneshot_reqrep::send_request;

use crate::ipc::{Command, SOCK_PATH};

pub async fn run(cmd: Command) -> Result<(), oneshot_reqrep::Error> {
    if let Err(e) = send_request(SOCK_PATH, cmd).await? {
        eprintln!("{}", e);
        std::process::exit(1);
    }

    Ok(())
}

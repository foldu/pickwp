use oneshot_reqrep::send_request;

use crate::ipc::{Command, Reply, SOCK_PATH};

pub async fn run(cmd: Command) -> Result<(), oneshot_reqrep::Error> {
    match send_request(SOCK_PATH, cmd).await? {
        Ok(Reply::Wps(wps)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&wps).expect("Can't deserialize to json")
            );
        }
        Ok(Reply::Unit) => {}
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

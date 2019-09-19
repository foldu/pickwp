use oneshot_reqrep::send_request;
use serde::{Deserialize, Serialize};

use crate::{
    config,
    ipc::{Command, Reply, SOCK_PATH},
    CmdConfig,
};

enum Formatter {
    Json,
    Yaml,
}

impl Formatter {
    fn print<T>(&self, t: &T)
    where
        T: Serialize,
    {
        let s = match self {
            Formatter::Json => serde_json::to_string_pretty(t).expect("Can't deserialize to json"),
            Formatter::Yaml => serde_yaml::to_string(t).expect("Can't deserialize to yaml"),
        };

        println!("{}", s);
    }
}

#[derive(Serialize, Deserialize)]
struct FilterWithId {
    id: usize,
    #[serde(flatten)]
    filter: config::Filter,
}

pub async fn run(cmd: Command, cmd_config: CmdConfig) -> Result<(), oneshot_reqrep::Error> {
    let formatter = if cmd_config.json {
        Formatter::Json
    } else {
        Formatter::Yaml
    };
    match send_request(SOCK_PATH, cmd).await? {
        Ok(Reply::Wps(wps)) => {
            formatter.print(&wps);
        }
        Ok(Reply::Filters(filters)) => {
            let filters: Vec<FilterWithId> = filters
                .into_iter()
                .enumerate()
                .map(|(i, filter)| FilterWithId { id: i, filter })
                .collect();
            formatter.print(&filters);
        }
        Ok(Reply::Unit) => {}
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

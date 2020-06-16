mod cfg;
mod cli;
mod client;
mod daemon;
mod data;
mod db;
mod monitor;
mod rpc;
mod scan;
mod unix;
mod util;
mod watch_file;

use crate::cli::Opt;
use structopt::StructOpt;

async fn run(opt: Opt) -> Result<(), anyhow::Error> {
    match opt.cmd {
        None => daemon::run().await?,
        Some(cmd) => client::run(cmd).await?,
    };
    Ok(())
}

fn init_global_logger() {
    // FIXME: hack for default log level=info
    match std::env::var_os("RUST_LOG") {
        Some(_) => (),
        None => {
            std::env::set_var("RUST_LOG", "info");
        }
    };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init()
}

fn main() {
    let opt = Opt::from_args();
    // need to use slog global logger because tokio spawns its own threads
    let _global_logger_guard = init_global_logger();

    let mut rt = tokio::runtime::Builder::new()
        .threaded_scheduler()
        .core_threads(2)
        .enable_all()
        .build()
        .unwrap();
    if let Err(e) = rt.block_on(run(opt)) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

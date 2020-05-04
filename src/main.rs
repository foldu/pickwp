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
use slog::Drain;
use structopt::StructOpt;

async fn run(opt: Opt) -> Result<(), anyhow::Error> {
    match opt.cmd {
        None => daemon::run().await?,
        Some(cmd) => client::run(cmd).await?,
    };
    Ok(())
}

fn init_global_logger() -> slog_scope::GlobalLoggerGuard {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = std::sync::Mutex::new(slog_term::FullFormat::new(decorator).build())
        .filter_level(slog::Level::Debug)
        .fuse();

    let logger = slog::Logger::root(drain, slog::o!());

    slog_scope::set_global_logger(logger)
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

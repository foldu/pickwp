#![recursion_limit = "1024"]

mod cache;
mod client;
mod config;
mod filter;
mod ipc;
mod macros;
mod monitor;
mod scan;
mod storage;
mod util;
mod watch_file;

use crate::{
    filter::Filter,
    ipc::{FilterCommand, Reply},
    monitor::{Mode, Monitor},
    storage::Storage,
    util::{preemptible_interval, PathBufExt},
};
use futures_util::{
    future::TryFutureExt,
    stream::{self, StreamExt},
};
use rand::prelude::*;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, io, path::Path, time::Duration};
use structopt::StructOpt;
use tokio::{
    signal::unix::{signal, Signal, SignalKind},
    task,
};

async fn run() -> Result<(), Error> {
    let opt = Opt::from_args();
    match opt.cmd {
        None => run_server().await?,
        Some(cmd) => client::run(cmd, opt.cmd_config).await?,
    }

    Ok(())
}

async fn run_server() -> Result<(), Error> {
    let commands = oneshot_reqrep::listen(ipc::SOCK_PATH, 16)?.fuse();
    tokio::pin!(commands);

    let config = config::Config::load_or_write_default().await?;

    let (watch_task, config_reload) =
        watch_file::watch_file(&*config::CONFIG_PATH, std::time::Duration::from_secs(5)).unwrap();
    tokio::task::spawn(watch_task);
    let mut config_reload = config_reload.fuse();

    // FIXME: unwrap
    let (scan_ctx, mut new_wp_rx) = scan::ScanCtx::new().unwrap();
    task::spawn(
        scan_ctx
            .scan(config.wp_dir.clone().into_string().unwrap())
            .unwrap()
            .map_err(|e| log::error!("{}", e)),
    );
    let mut state = State::from_config(config, &scan_ctx.get_cache())
        .map_err(|e| Error::FilterCreate { src: e })?;

    let mut storage = Storage::default();
    storage.refresh(new_wp_rx.next().await.unwrap(), &scan_ctx.get_cache());
    let mut new_wp_rx = new_wp_rx.fuse();

    let (mut refresh_preempt, refresh) =
        preemptible_interval(Duration::from_secs(state.refresh_interval));
    let mut refresh = refresh.fuse();

    let (mut rescan_preempt, rescan) =
        preemptible_interval(Duration::from_secs(state.rescan_interval));
    let mut rescan = rescan.fuse();

    let int = register_signal(SignalKind::interrupt())?;
    let term = register_signal(SignalKind::terminate())?;
    let mut terminate = stream::select(int, term).fuse();

    set_wallpapers(&mut state, &storage)?;
    loop {
        tokio::select! {
            _ = refresh.next() => {
                if !state.frozen {
                    log::info!("Refreshing");
                    set_wallpapers(&mut state, &storage)?;
                }
            }
            buf = config_reload.next() => {
                let buf = buf.expect("Config reload died");
                match config::Config::load_from_buf(&buf) {
                    Ok(cfg) => {
                        log::info!("Reloaded config");
                        match State::from_config(cfg, &scan_ctx.get_cache()) {
                            Ok(new_state) => {
                                refresh_preempt.preempt().await;
                                state = new_state;
                            }

                            Err(e) => log::error!("{}", e),
                        }
                    }
                    Err(e) => {
                        log::error!("{}", e);
                    }
                }
            }
            _ = rescan.next() => {
                log::info!("Starting rescan");
                let wp_dir = state.wp_dir.clone();
                if let Some(task) = scan_ctx.scan(wp_dir) {
                    task::spawn(task);
                }
            }
            new_wps = new_wp_rx.next() => {
                if let Some(new_wps) = new_wps {
                    let cache= scan_ctx.get_cache();
                    storage.refresh(new_wps, &cache);
                }
            }
            _ = terminate.next() => {
                break Ok(());
            }
            req = commands.next() => {
                if let Some(req) = req {
                    log::debug!("Received cmd {:#?}", req.kind());
                    use ipc::Command::*;
                    let rep = match req.kind() {
                        Refresh => {
                            refresh_preempt.preempt().await;
                            Ok(Reply::Unit)
                        }
                        Rescan => {
                            rescan_preempt.preempt().await;
                            Ok(Reply::Unit)
                        }
                        Current => {
                            Ok(Reply::Wps(state.current.clone()))
                        }
                        Filters { action } => {
                            match action {
                                None => {
                                    Ok(Reply::Filters(state.filters.iter().map(|filter| filter.serializeable()).collect()))
                                }
                                Some(FilterCommand::Rm { id }) => {
                                    if *id < state.filters.len() {
                                        state.filters.remove(*id);
                                        Ok(Reply::Unit)
                                    } else {
                                        Err(format!("No filter with id {}", id))
                                    }
                                }
                                Some(FilterCommand::Add { filters }) => {
                                    state.filters.extend(filters.into_iter().map(|filter| filter.clone().into()));
                                    Ok(Reply::Unit)
                                }
                            }
                        }
                        ToggleFreeze => {
                            state.frozen = !state.frozen;
                            Ok(Reply::FreezeStatus(state.frozen))
                        }
                    };

                    try_or_err!(req.reply(&rep).await);
                }
            }
        }
    }
}

struct State {
    filters: Vec<Box<dyn Filter>>,
    wp_dir: String,
    mode: Mode,
    rescan_interval: u64,
    refresh_interval: u64,
    current: HashMap<String, Option<String>>,
    monitor: Box<dyn Monitor>,
    frozen: bool,
}

impl State {
    fn from_config(
        config: config::Config,
        cache: &crate::cache::Cache,
    ) -> Result<Self, crate::filter::FilterCreateError> {
        let filters: Vec<Box<dyn Filter>> = config
            .filters
            .into_iter()
            .map(|filter| filter.into())
            .map(|filter: Box<dyn Filter>| match filter.read_ctx(cache) {
                Ok(Some(new)) => Ok(new),
                Ok(None) => Ok(filter),
                Err(e) => Err(e),
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            frozen: false,
            filters,
            current: Default::default(),
            wp_dir: config.wp_dir.into_string().unwrap(),
            mode: config.mode,
            refresh_interval: config.refresh_interval,
            rescan_interval: config.refresh_interval,
            monitor: config.backend.into(),
        })
    }
}

#[derive(StructOpt, Debug)]
struct Opt {
    #[structopt(subcommand)]
    cmd: Option<ipc::Command>,
    #[structopt(flatten)]
    cmd_config: CmdConfig,
}

#[derive(StructOpt, Debug)]
pub struct CmdConfig {
    /// Format output in json
    #[structopt(short, long)]
    json: bool,
}

fn register_signal(kind: SignalKind) -> Result<Signal, Error> {
    signal(kind).context(RegisterSignal)
}

fn set_wallpapers(state: &mut State, storage: &Storage) -> Result<(), Error> {
    let mut rng = rand::thread_rng();

    let filtered = storage
        .keys()
        .filter(|key| {
            state
                .filters
                .iter_mut()
                .all(|filter| filter.is_ok(*key, storage))
        })
        .collect::<Vec<_>>();

    let mut new = Vec::new();
    state.current.clear();
    let screens = state.monitor.idents()?;
    for screen in screens {
        let path = if let Some(pick) = filtered.choose(&mut rng) {
            new.push(*pick);
            let path = Path::new(&state.wp_dir)
                .join(storage.relative_paths.get(*pick).unwrap().as_str())
                .into_string()
                .unwrap();

            state.monitor.set_wallpaper(state.mode, &screen, &path)?;

            Some(path)
        } else {
            None
        };

        state.current.insert(screen, path);
    }

    for filter in &mut state.filters {
        filter.after_wp_refresh(&new);
    }

    Ok(())
}

#[derive(Snafu, Debug)]
enum Error {
    #[snafu(display("Can't register signal handler: {}", source))]
    RegisterSignal { source: io::Error },

    #[snafu(display("{}", source))]
    #[snafu(context(false))]
    Ipc { source: oneshot_reqrep::Error },

    #[snafu(context(false))]
    Config { source: config::Error },

    #[snafu(context(false))]
    MonitorErr { source: monitor::Error },

    #[snafu(display("Can't create filter: {}", src))]
    FilterCreate { src: filter::FilterCreateError },
}

fn main() {
    let log = "RUST_LOG";
    if let Err(_) = std::env::var(log) {
        std::env::set_var("RUST_LOG", "pickwp=info");
    }
    env_logger::init();
    let mut rt = tokio::runtime::Builder::new()
        .basic_scheduler()
        .enable_io()
        .enable_time()
        .core_threads(2)
        .build()
        .unwrap();

    if let Err(e) = rt.block_on(run()) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

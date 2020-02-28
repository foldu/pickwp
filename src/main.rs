#![recursion_limit = "1024"]

mod cache;
mod cli;
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
    cli::{FilterCommand, Opt},
    filter::Filter,
    ipc::Reply,
    monitor::{Mode, Monitor},
    storage::Storage,
    util::{preemptible_interval, PathBufExt},
};
use futures_util::{
    future::TryFutureExt,
    stream::{self, StreamExt},
};
use rand::prelude::*;
use slog_scope::{debug, error, info};
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
    let commands = oneshot_reqrep::listen(ipc::SOCK_PATH, 16)?;
    tokio::pin!(commands);

    let config = config::Config::load_or_write_default().await?;

    let (watch_task, mut config_reload) =
        watch_file::watch_file(&*config::CONFIG_PATH, std::time::Duration::from_secs(5)).unwrap();
    task::spawn(watch_task);

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
    storage
        .refresh(new_wp_rx.next().await.unwrap(), &scan_ctx.get_cache())
        .context(RefreshStorage)?;
    let mut new_wp_rx = new_wp_rx;

    let (mut refresh_preempt, mut refresh) =
        preemptible_interval(Duration::from_secs(state.refresh_interval));

    let (mut rescan_preempt, mut rescan) =
        preemptible_interval(Duration::from_secs(state.rescan_interval));

    let int = register_signal(SignalKind::interrupt())?;
    let term = register_signal(SignalKind::terminate())?;
    let mut terminate = stream::select(int, term);

    refresh_preempt.preempt().await;
    loop {
        tokio::select! {
            Some(_) = refresh.next() => {
                if !state.frozen {
                    if let Err(e) = set_wallpapers(&mut state, &storage).await {
                        error!("{}", e);
                    }
                }
            }
            Some(buf) = config_reload.next() => {
                match config::Config::load_from_buf(&buf) {
                    Ok(cfg) => {
                        info!("Reloaded config");
                        match State::from_config(cfg, &scan_ctx.get_cache()) {
                            Ok(new_state) => {
                                refresh_preempt.preempt().await;
                                state = new_state;
                            }

                            Err(e) => error!("Could not create new state from config"; slog::o!("source" => e.to_string())),
                        }
                    }
                    Err(e) => {
                        error!("Could not load config"; slog::o!("source" => e.to_string()));
                    }
                }
            }
            Some(_) = rescan.next() => {
                info!("Starting rescan");
                let wp_dir = state.wp_dir.clone();
                if let Some(task) = scan_ctx.scan(wp_dir) {
                    task::spawn(task);
                }
            }
            Some(new_wps) = new_wp_rx.next() => {
                let cache= scan_ctx.get_cache();
                if let Err(e) = storage.refresh(new_wps, &cache) {
                    error!("Error refreshing storage"; slog::o!("source" => e.to_string()));
                }
            }
            Some(_) = terminate.next() => {
                break Ok(());
            }
            Some(req) = commands.next() => {
                debug!("Received cmd"; slog::o!("kind" => format!("{:#?}", req.kind())));
                use cli::Command::*;
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
                            //Some(FilterCommand::Add { filters }) => {
                            //    state.filters.extend(filters.into_iter().map(|filter| filter.clone().into()));
                            //    Ok(Reply::Unit)
                            //}
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

fn register_signal(kind: SignalKind) -> Result<Signal, Error> {
    signal(kind).context(RegisterSignal)
}

async fn set_wallpapers(state: &mut State, storage: &Storage) -> Result<(), Error> {
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
    let screens = state.monitor.idents().await?;
    for screen in screens {
        let path = if let Some(pick) = filtered.choose(&mut rng) {
            new.push(*pick);
            let path = Path::new(&state.wp_dir)
                .join(storage.relative_paths.get(*pick).unwrap().as_str())
                .into_string()
                .unwrap();

            state
                .monitor
                .set_wallpaper(state.mode, &screen, &path)
                .await?;

            Some(path)
        } else {
            None
        };

        match &path {
            Some(path) => {
                info!("Set wp"; slog::b!("screen" => &screen, "path" => &path));
            }
            None => {
                info!("No wp found"; slog::b!("screen" => &screen));
            }
        }
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

    #[snafu(display("Could not refresh storage: {}", source))]
    RefreshStorage { source: cache::Error },
}

fn main() {
    use slog::Drain;
    let decorator = slog_term::TermDecorator::new().build();
    let drain = std::sync::Mutex::new(slog_term::FullFormat::new(decorator).build())
        .filter_level(slog::Level::Info)
        .fuse();

    let logger = slog::Logger::root(drain, slog::o!());

    let _scope_guard = slog_scope::set_global_logger(logger);
    let _log_guard = slog_stdlog::init().unwrap();

    let mut rt = tokio::runtime::Builder::new()
        .basic_scheduler()
        .enable_io()
        .enable_time()
        .core_threads(2)
        .build()
        .unwrap();

    if let Err(e) = rt.block_on(run()) {
        slog_scope::crit!("{}", e);
        std::process::exit(1);
    }
}

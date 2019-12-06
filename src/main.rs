#![recursion_limit = "1024"]

mod client;
mod config;
mod filter;
mod ipc;
mod macros;
mod monitor;
mod storage;
mod util;

use crate::{
    filter::Filter,
    ipc::{FilterCommand, Reply},
    monitor::{Mode, Monitor},
    storage::{RelativePath, Storage, StorageFlags},
    util::{preemptible_interval, PathBufExt},
};
use cfgen::prelude::*;
use futures_util::{
    pin_mut,
    stream::{self, StreamExt},
};
use rand::prelude::*;
use snafu::{ResultExt, Snafu};
use std::{
    collections::HashMap,
    convert::TryFrom,
    io,
    os::unix::prelude::*,
    path::Path,
    time::Duration,
};
use structopt::StructOpt;
use tokio::{
    signal::unix::{signal, Signal, SignalKind},
    sync::mpsc::{self},
    task,
};
use walkdir::{DirEntry, WalkDir};

async fn run() -> Result<(), Error> {
    let opt = Opt::from_args();
    match opt.cmd {
        None => run_server().await?,
        Some(cmd) => client::run(cmd, opt.cmd_config).await.context(Ipc)?,
    }

    Ok(())
}

async fn run_server() -> Result<(), Error> {
    let commands = oneshot_reqrep::listen(ipc::SOCK_PATH, 16)
        .context(Ipc)?
        .fuse();
    pin_mut!(commands);

    let (_, config) = config::Config::load_or_write_default().context(Config)?;

    let mut state = State::from_config(config);

    let mut storage = Storage::default();
    let wps: Vec<_> = get_wallpapers(state.wp_dir.to_string(), state.needed).await;
    storage.refresh(wps);

    let (mut refresh_preempt, refresh) =
        preemptible_interval(Duration::from_secs(state.refresh_interval));
    let mut refresh = refresh.fuse();

    let (mut rescan_preempt, rescan) =
        preemptible_interval(Duration::from_secs(state.rescan_interval));
    let mut rescan = rescan.fuse();

    let int = register_signal(SignalKind::interrupt())?;
    let term = register_signal(SignalKind::terminate())?;
    let mut terminate = stream::select(int, term).fuse();

    let (new_wp_tx, new_wp_rx) = mpsc::channel(1);
    let mut new_wp_rx = new_wp_rx.fuse();

    set_wallpapers(&mut state, &storage)?;
    loop {
        futures_util::select! {
            _ = refresh.next() => {
                if !state.frozen {
                    log::info!("Refreshing");
                    set_wallpapers(&mut state, &storage)?;
                }
            }
            _ = rescan.next() => {
                log::info!("Starting rescan");
                let wp_dir = state.wp_dir.clone();
                let needed = state.needed;
                let mut new_wp_tx = new_wp_tx.clone();
                task::spawn(async move {
                    let wps: Vec<_> = get_wallpapers( wp_dir, needed).await;
                    new_wp_tx.send(wps).await.unwrap();
                });
            }
            new_wps = new_wp_rx.next() => {
                if let Some(new_wps) = new_wps {
                    storage.refresh(new_wps);
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
                        ReloadConfig => {
                            match config::Config::load() {
                                Ok(config) => {
                                    // FIXME: not all things are properly reset like
                                    // {rescan,refresh}_interval
                                    state = State::from_config(config);
                                    log::info!("Reloaded config");
                                    Ok(Reply::Unit)
                                }
                                Err(e) => {
                                   Err(e.to_string())
                                }
                            }
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
    needed: StorageFlags,
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
    fn from_config(config: config::Config) -> Self {
        let filters: Vec<Box<dyn Filter>> = config
            .filters
            .into_iter()
            .map(|filter| filter.into())
            .collect();
        let needed = filters
            .iter()
            .map(|filter| filter.needed_storages())
            .fold(StorageFlags::NONE, |flags, flag| flag | flags);

        Self {
            frozen: false,
            filters,
            current: Default::default(),
            needed,
            wp_dir: config.wp_dir.0.into_string().unwrap(),
            mode: config.mode,
            refresh_interval: config.refresh_interval,
            rescan_interval: config.refresh_interval,
            monitor: config.backend.into(),
        }
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
    let screens = state.monitor.idents().context(MonitorErr)?;
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
                .context(MonitorErr)?;

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

async fn get_wallpapers(
    dir: String,
    needed: StorageFlags,
) -> Vec<(RelativePath, Option<storage::Time>)> {
    task::spawn_blocking(move || {
        let mut ret = Vec::new();
        WalkDir::new(&dir).into_iter().for_each(|entry| {
            if let Ok(entry) = entry {
                let is_image = is_image(&entry);
                if is_image {
                    let time = if needed.contains(StorageFlags::FILETIME) {
                        entry
                            .metadata()
                            .map(|meta| Some(storage::Time::from_meta(&meta)))
                    } else {
                        Ok(None)
                    };
                    let relative = {
                        let unprefixed = entry.path().strip_prefix(&dir).unwrap();
                        RelativePath::try_from(unprefixed.to_owned())
                    };

                    if let (Ok(relative), Ok(time)) = (relative, time) {
                        ret.push((relative, time));
                    }
                }
            }
        });
        ret
    })
    .await
    .unwrap_or_else(|_| Vec::new())
}

static IMAGE_EXTENSIONS: phf::Set<&'static [u8]> = phf::phf_set! {
    b"jpe",
    b"jpeg",
    b"jpg",
    b"png",
};

fn is_image(ent: &DirEntry) -> bool {
    ent.file_type().is_file()
        && ent
            .path()
            .extension()
            .map(|ext| IMAGE_EXTENSIONS.contains(ext.as_bytes()))
            .unwrap_or(false)
}

#[derive(Snafu, Debug)]
enum Error {
    #[snafu(display("Can't register signal handler: {}", source))]
    RegisterSignal {
        source: io::Error,
    },

    #[snafu(display("Can't decode json received from swaymsg: {}", source))]
    Json {
        source: serde_json::Error,
    },

    #[snafu(display("{}", source))]
    Ipc {
        source: oneshot_reqrep::Error,
    },

    Config {
        source: cfgen::Error,
    },

    MonitorErr {
        source: monitor::Error,
    },
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
        .build()
        .unwrap();

    if let Err(e) = rt.block_on(run()) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

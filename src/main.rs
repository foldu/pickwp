#![recursion_limit = "1024"]
#![feature(proc_macro_hygiene)]

mod client;
mod config;
mod filter;
mod ipc;
mod storage;
mod util;

use std::{
    collections::HashMap,
    convert::TryFrom,
    io,
    os::unix::prelude::*,
    path::Path,
    process::Command,
    thread,
    time::Duration,
};

use cfgen::prelude::*;
use futures::{pin_mut, prelude::*, stream};
use rand::prelude::*;
use serde::Deserialize;
use snafu::{ResultExt, Snafu};
use structopt::StructOpt;
use tokio::{
    runtime::current_thread,
    sync::mpsc::{self, Receiver},
};
use tokio_net::signal::unix::{signal, Signal, SignalKind};
use walkdir::{DirEntry, WalkDir};

use crate::{
    config::Mode,
    filter::Filter,
    ipc::Reply,
    storage::{RelativePath, Storage, StorageFlags},
    util::{preemptible_interval, PathBufExt},
};

async fn run(handle: current_thread::Handle) -> Result<(), Error> {
    let opt = Opt::from_args();
    match opt {
        Opt::Daemon => run_server(handle).await?,
        Opt::Client(cmd) => client::run(cmd).await.context(Ipc)?,
    }

    Ok(())
}

async fn run_server(handle: current_thread::Handle) -> Result<(), Error> {
    std::env::set_var("RUST_LOG", "pickwp=info");
    let commands = oneshot_reqrep::listen(ipc::SOCK_PATH).context(Ipc)?.fuse();
    pin_mut!(commands);

    let (_, config) = config::Config::load_or_write_default().context(Config)?;

    let mut state = State::from_config(config);

    let mut storage = Storage::new();
    let wps: Vec<_> = get_wallpapers(handle.clone(), state.wp_dir.to_string(), state.needed)
        .collect()
        .await;
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
        futures::select! {
            _ = refresh.next() => {
                log::debug!("refresh");
                set_wallpapers(&mut state, &storage)?;
            }
            _ = rescan.next() => {
                log::debug!("rescan");
                let wp_dir = state.wp_dir.clone();
                let handle = handle.clone();
                let needed = state.needed;
                let mut new_wp_tx = new_wp_tx.clone();
                current_thread::spawn(async move {
                    let wps: Vec<_> = get_wallpapers(handle, wp_dir, needed).collect().await;
                    let _ = new_wp_tx.send(wps).await;
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
                    match req.kind() {
                        Refresh => {
                            let _ = refresh_preempt.preempt().await;
                            let _ = req.reply(&Ok(Reply::Unit)).await;
                        }
                        Rescan => {
                            let _ = rescan_preempt.preempt().await;
                            let _ = req.reply(&Ok(Reply::Unit)).await;
                        }
                        ReloadConfig => {
                            log::error!("config reload not implemented");
                            match config::Config::load() {
                                Ok(config) => {
                                    // FIXME: not all things are properly reset like
                                    // {rescan,refresh}_interval
                                    state = State::from_config(config);
                                    let _ = req.reply(&Ok(Reply::Unit)).await;
                                }
                                Err(e) => {
                                    let _ = req.reply(&Err(e.to_string())).await;
                                }
                            }
                        }
                        Current => {
                            let _ = req.reply(&Ok(Reply::Wps(state.current.clone()))).await;
                        }
                    };
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
            filters,
            current: Default::default(),
            needed,
            wp_dir: config.wp_dir.0.into_string().unwrap(),
            mode: config.mode,
            refresh_interval: config.refresh_interval,
            rescan_interval: config.refresh_interval,
        }
    }
}

#[derive(StructOpt, Debug)]
enum Opt {
    #[structopt(name = "daemon")]
    Daemon,

    #[structopt(name = "send")]
    Client(ipc::Command),
}

fn register_signal(kind: SignalKind) -> Result<Signal, Error> {
    signal(kind).context(RegisterSignal)
}

struct Screen {
    ident: String,
}

#[derive(Deserialize)]
struct GetOutputField {
    name: String,
}

fn get_outputs() -> Result<impl Iterator<Item = Screen>, Error> {
    let output = Command::new("swaymsg")
        .arg("-rt")
        .arg("get_outputs")
        .output()
        .context(SwaymsgLaunch)?;

    let parsed: Vec<GetOutputField> = serde_json::from_slice(&output.stdout).context(Json)?;

    Ok(parsed.into_iter().map(|field| Screen { ident: field.name }))
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
    for screen in get_outputs()? {
        let path = if let Some(pick) = filtered.choose(&mut rng) {
            new.push(*pick);
            let path = Path::new(&state.wp_dir)
                .join(storage.relative_paths.get(*pick).unwrap().as_str())
                .into_string()
                .unwrap();
            let arg = format!(
                r#"output {} background "{}" {}"#,
                screen.ident, path, state.mode
            );

            Command::new("swaymsg")
                .arg(arg)
                .output()
                .context(SwaymsgLaunch)?;
            Some(path)
        } else {
            None
        };

        state.current.insert(screen.ident, path);
    }

    for filter in &mut state.filters {
        filter.after_wp_refresh(&new);
    }

    Ok(())
}

fn get_wallpapers(
    handle: current_thread::Handle,
    dir: String,
    needed: StorageFlags,
) -> Receiver<(RelativePath, Option<storage::Time>)> {
    let (tx, rx) = mpsc::channel(64);
    thread::spawn(move || {
        WalkDir::new(&dir).into_iter().for_each(move |entry| {
            if let Ok(entry) = entry {
                let is_image = is_image(&entry);
                if is_image {
                    let mut tx = tx.clone();
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
                        let _ = handle.spawn(async move {
                            let _ = tx.send((relative, time)).await;
                        });
                    }
                }
            }
        });
    });

    rx
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

    #[snafu(display("Can't launch swaymsg: {}", source))]
    SwaymsgLaunch {
        source: io::Error,
    },

    #[snafu(display("Can't decode json received from swaymsg: {}", source))]
    Json {
        source: serde_json::Error,
    },

    #[snafu(display("Error while doing ipc: {}", source))]
    Ipc {
        source: oneshot_reqrep::Error,
    },

    Config {
        source: cfgen::Error,
    },
}

fn main() {
    env_logger::init();
    let mut rt = current_thread::Runtime::new().unwrap();
    let handle = rt.handle();
    if let Err(e) = rt.block_on(run(handle)) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

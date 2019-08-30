#![recursion_limit = "1024"]
#![feature(proc_macro_hygiene)]

mod client;
mod config;
mod filter;
mod ipc;

use std::{io, os::unix::prelude::*, process::Command, thread, time::Duration};

use cfgen::prelude::*;
use futures::{pin_mut, prelude::*, stream};
use rand::prelude::*;
use serde::Deserialize;
use snafu::{ResultExt, Snafu};
use structopt::StructOpt;
use strum_macros::EnumString;
use tokio::{
    runtime::current_thread,
    sync::mpsc::{self, Receiver},
    timer::Interval,
};
use tokio_net::signal::unix::{signal, Signal, SignalKind};
use walkdir::{DirEntry, WalkDir};

use crate::filter::Filter;

async fn run(handle: current_thread::Handle) -> Result<(), Error> {
    let opt = Opt::from_args();
    match opt {
        Opt::Daemon(opt) => run_server(handle, opt).await?,
        Opt::Client(cmd) => client::run(cmd).await.context(Ipc)?,
    }

    Ok(())
}

async fn run_server(handle: current_thread::Handle, opt: DaemonOpt) -> Result<(), Error> {
    let commands = oneshot_reqrep::listen(ipc::SOCK_PATH).context(Ipc)?.fuse();
    pin_mut!(commands);

    let (_, config) = config::Config::load_or_write_default().context(Config)?;

    let wp_dir = match opt.wp_dir {
        Some(ref dir) => dir.clone(),
        None => config.wp_dir.0.to_str().unwrap().to_string(),
    };

    log::info!("Using {} as wp dir", wp_dir);

    let mut wps: Vec<String> = get_wallpapers(handle.clone(), wp_dir.to_string())
        .collect()
        .await;

    let (refresh_preempt, refresh) =
        preemptible_interval(Duration::from_secs(opt.refresh_interval));
    let mut refresh = refresh.fuse();

    let (rescan_preempt, rescan) = preemptible_interval(Duration::from_secs(opt.rescan_interval));
    let mut rescan = rescan.fuse();

    let int = register_signal(SignalKind::interrupt())?;
    let term = register_signal(SignalKind::terminate())?;
    let mut terminate = stream::select(int, term).fuse();

    let (new_wp_tx, new_wp_rx) = mpsc::channel(1);
    let mut new_wp_rx = new_wp_rx.fuse();

    let mut filters: Vec<Box<dyn Filter>> = vec![Box::new(filter::LastShown::default())];

    set_wallpapers(&mut filters, &wps, opt.mode)?;
    loop {
        futures::select! {
            _ = refresh.next() => {
                log::debug!("refresh");
                set_wallpapers(&mut filters, &wps, opt.mode)?;
            }
            _ = rescan.next() => {
                log::debug!("rescan");
                let wp_dir = wp_dir.clone();
                let handle = handle.clone();
                let mut new_wp_tx = new_wp_tx.clone();
                current_thread::spawn(async move {
                    let wps = get_wallpapers(handle.clone(), wp_dir.to_string()).collect().await;
                    let _ = new_wp_tx.send(wps).await;
                });
            }
            new_wps = new_wp_rx.next() => {
                if let Some(new_wps) = new_wps {
                    wps = new_wps;
                }
            }
            _ = terminate.next() => {
                break Ok(());
            }
            req = commands.next() => {
                if let Some(req) = req {
                    log::debug!("Received cmd {:#?}", req.kind());
                    let mut refresh_preempt = refresh_preempt.clone();
                    let mut rescan_preempt = rescan_preempt.clone();
                    let _ = handle.spawn(async move {
                        use ipc::Command::*;
                        match req.kind() {
                            Refresh => {
                                let _ = refresh_preempt.preempt().await;
                                let _ = req.reply(()).await;
                            }
                            Rescan => {
                                let _ = rescan_preempt.preempt().await;
                                let _ = req.reply(()).await;
                            }
                        }
                    });

                }
            }
        }
    }
}

#[derive(StructOpt, Debug)]
struct DaemonOpt {
    #[structopt(long = "wp-dir")]
    wp_dir: Option<String>,

    #[structopt(long = "rescan-interval", default_value = "600")]
    rescan_interval: u64,

    #[structopt(long = "refresh-interval", default_value = "300")]
    refresh_interval: u64,

    #[structopt(long = "mode", default_value = "Fill")]
    mode: Mode,
}

#[derive(StructOpt, Debug)]
enum Opt {
    #[structopt(name = "daemon")]
    Daemon(DaemonOpt),

    #[structopt(name = "send")]
    Client(ipc::Command),
}

#[derive(Clone, Debug)]
struct Preempter {
    tx: mpsc::Sender<()>,
}

impl Preempter {
    async fn preempt(&mut self) -> Result<(), tokio::sync::mpsc::error::SendError> {
        self.tx.send(()).await
    }
}

fn preemptible_interval(timeout: Duration) -> (Preempter, impl Stream<Item = ()>) {
    let (preempt_tx, preempt_rx) = mpsc::channel(4);

    let (mut inner_tx, inner_rx) = mpsc::channel(4);
    current_thread::spawn(async move {
        let mut interval = Interval::new_interval(timeout).fuse();
        let mut preempt_rx = preempt_rx.fuse();
        loop {
            futures::select! {
                _ = interval.next() => {
                }
                preemption = preempt_rx.next() => {
                    // preempter dropped, time to stop
                    if let None = preemption {
                        break;
                    }
                    interval = Interval::new_interval(timeout).fuse();
                }
            }

            let _ = inner_tx.send(()).await;
        }
    });

    (Preempter { tx: preempt_tx }, inner_rx)
}

fn register_signal(kind: SignalKind) -> Result<Signal, Error> {
    signal(kind).context(RegisterSignal)
}

#[derive(EnumString, Copy, Debug, Clone)]
enum Mode {
    Fill,

    Tile,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            Mode::Fill => "fill",
            Mode::Tile => "tile",
        };
        fmt.write_str(s)
    }
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

fn set_wallpapers(
    filters: &mut [Box<dyn Filter>],
    wps: &[String],
    mode: Mode,
) -> Result<(), Error> {
    let mut rng = rand::thread_rng();

    let filtered = wps
        .iter()
        .filter(|wp| filters.iter_mut().any(|filter| !filter.is_filtered(wp)))
        .collect::<Vec<_>>();

    let mut new = Vec::new();
    for screen in get_outputs()? {
        if let Some(pick) = filtered.choose(&mut rng) {
            new.push(pick.as_str());
            let arg = format!(r#"output {} background "{}" {}"#, screen.ident, pick, mode);
            Command::new("swaymsg")
                .arg(arg)
                .output()
                .context(SwaymsgLaunch)?;
        }
    }

    for filter in filters {
        filter.after_wp_refresh(&new);
    }

    Ok(())
}

fn get_wallpapers(handle: current_thread::Handle, dir: String) -> Receiver<String> {
    let (tx, rx) = mpsc::channel(64);
    thread::spawn(move || {
        WalkDir::new(dir).into_iter().for_each(move |entry| {
            if let Ok(entry) = entry {
                let is_image = is_image(&entry);
                let s = entry.into_path().into_os_string().into_string();

                match (s, is_image) {
                    (Ok(s), true) => {
                        let mut tx = tx.clone();
                        let _ = handle.spawn(async move {
                            let _ = tx.send(s).await;
                        });
                    }
                    _ => {}
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

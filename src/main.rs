#![recursion_limit = "256"]
#![feature(async_await, proc_macro_hygiene)]

mod client;
mod command;

use std::{io, os::unix::prelude::*, path::PathBuf, process::Command, thread, time::Duration};

use derive_more::Display;
use futures::{prelude::*, stream};
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
use tokio_signal::unix::{Signal, SIGINT, SIGTERM};
use walkdir::{DirEntry, WalkDir};

use command::command_stream;

async fn run(handle: current_thread::Handle) -> Result<(), Error> {
    let opt = Opt::from_args();
    match opt {
        Opt::Daemon(opt) => run_server(handle, opt).await?,
        Opt::Client(cmd) => client::run(cmd).await.context(Ipc)?,
    }

    Ok(())
}

async fn run_server(handle: current_thread::Handle, opt: DaemonOpt) -> Result<(), Error> {
    let commands = command_stream().context(Ipc)?.fuse();
    futures::pin_mut!(commands);

    let mut wps: Vec<String> = get_wallpapers(handle.clone(), opt.wp_dir.clone())
        .collect()
        .await;

    let refresh = Interval::new_interval(Duration::from_secs(opt.refresh_interval)).map(|_| ());
    let (refresh_manual_tx, refresh_manual_rx) = mpsc::channel(4);
    let mut refresh = stream::select(refresh, refresh_manual_rx).fuse();

    let mut rescan = Interval::new_interval(Duration::from_secs(opt.rescan_interval)).fuse();

    let int = register_signal(SIGINT)?;
    let term = register_signal(SIGTERM)?;
    let mut terminate = stream::select(int, term).fuse();

    let (new_wp_tx, new_wp_rx) = mpsc::channel(1);
    let mut new_wp_rx = new_wp_rx.fuse();

    set_wallpapers(&wps, opt.mode)?;
    loop {
        futures::select! {
            _ = refresh.next() => {
                log::debug!("refresh");
                set_wallpapers(&wps, opt.mode)?;
            }
            _ = rescan.next() => {
                log::debug!("rescan");
                let wp_dir = opt.wp_dir.clone();
                let handle = handle.clone();
                let mut new_wp_tx = new_wp_tx.clone();
                current_thread::spawn(async move {
                    let wps = get_wallpapers(handle.clone(), wp_dir).collect().await;
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
                    let mut tx = refresh_manual_tx.clone();
                    let _ = handle.spawn(async move {
                        use command::Command::*;
                        match req.cmd {
                            Refresh => {
                                let _ = tx.send(()).await;
                                let _ = req.reply(command::Reply::Ok).await;
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
    wp_dir: PathBuf,

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
    Client(client::Subcmd),
}

fn register_signal(signo: i32) -> Result<Signal, Error> {
    Signal::new(signo).context(RegisterSignal)
}

#[derive(Display, EnumString, Copy, Debug, Clone)]
enum Mode {
    #[display(fmt = "fill")]
    Fill,

    #[display(fmt = "tile")]
    Tile,
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

fn set_wallpapers(wps: &[String], mode: Mode) -> Result<(), Error> {
    let mut rng = rand::thread_rng();

    for screen in get_outputs()? {
        if let Some(pick) = wps.choose(&mut rng) {
            let arg = format!(r#"output {} background "{}" {}"#, screen.ident, pick, mode);
            Command::new("swaymsg")
                .arg(arg)
                .output()
                .context(SwaymsgLaunch)?;
        }
    }

    Ok(())
}

fn get_wallpapers(handle: current_thread::Handle, dir: PathBuf) -> Receiver<String> {
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
    RegisterSignal { source: io::Error },

    #[snafu(display("Can't launch swaymsg: {}", source))]
    SwaymsgLaunch { source: io::Error },

    #[snafu(display("Can't decode json received from swaymsg: {}", source))]
    Json { source: serde_json::Error },

    #[snafu(display("Error while doing ipc: {}", source))]
    Ipc { source: command::IpcError },
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

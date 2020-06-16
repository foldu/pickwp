use crate::{
    cfg::Config,
    db::{self, RootData},
    monitor::Monitor,
    rpc,
    scan::ImageScanner,
    util::Preempter,
    watch_file::FileWatcher,
};
use futures_util::stream::{self, Stream, StreamExt};
use snafu::ResultExt;
use std::{collections::BTreeMap, sync::Arc};
use tokio::{
    signal::unix::{signal, SignalKind},
    sync::Mutex,
    task,
};

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    #[snafu(context(false), display("{}", source))]
    Bind { source: rpc::ServerError },

    #[snafu(context(false), display("{}", source))]
    DbOpen { source: db::OpenError },

    #[snafu(display("Can't register signal: {}", source))]
    RegisterSignal { source: std::io::Error },

    #[snafu(display("Could not set wallpapers: {}", source))]
    #[snafu(context(false))]
    Monitor { source: crate::monitor::Error },

    #[snafu(context(false), display("sqlite error: {}", source))]
    Db { source: sqlx::Error },
}

pub async fn run() -> Result<(), Error> {
    let app_paths = crate::util::AppPaths::get().unwrap();

    let mut cfg = Config::load_or_write_default(&app_paths.config_file).unwrap();

    let (watch_task, mut cfg_reload) = FileWatcher::default().watch(app_paths.config_file).unwrap();
    task::spawn(watch_task);

    let state = State::default();
    let server = rpc::bind(app_paths.rt_dir)?;
    task::spawn(server.serve(state.clone()));

    let pool = db::open(&app_paths.db_file).await?;
    let mut term = signal_stream(&[SignalKind::terminate(), SignalKind::interrupt()])?;

    let mut image_scanner = crate::scan::ImageScanner::new();

    // FIXME: make configurable
    let mut mon: Box<dyn Monitor> = Box::new(crate::monitor::Sway::default());

    loop {
        let root = {
            // FIXME: this doesn't work if scan is running
            let mut cxn = pool.acquire().await.unwrap();
            db::get_or_insert_root(&mut cxn, cfg.wp_dir.clone()).await?
        };

        image_scanner.abort_if_root_differs(root.id()).await;

        let loop_ = ControlLoop {
            cfg_reload: &mut cfg_reload,
            terminate: &mut term,
            image_scanner: &mut image_scanner,
            pool: &pool,
            cfg: &cfg,
            state: &state,
            mon: &mut *mon,
            root,
        };

        tracing::debug!("Starting event loop");

        match loop_.run().await {
            Ok(LoopExit::Terminate) => break Ok(()),
            Ok(LoopExit::NewCfg(new_cfg)) => {
                cfg = new_cfg;
            }
            Err(e) => {
                break Err(e);
            }
        }

        state.clear().await;
    }
}

#[derive(Default, Clone, derive_more::Deref)]
pub struct State(Arc<Mutex<Option<StateInner>>>);

impl State {
    pub async fn store(&self, inner: StateInner) {
        *self.0.lock().await = Some(inner);
    }

    pub async fn clear(&self) {
        *self.0.lock().await = None;
    }
}

#[derive(Debug)]
pub struct StateInner {
    pub current_wps: BTreeMap<String, Option<String>>,
    pub frozen: bool,
    pub scan_preempt: Preempter,
    pub refresh_preempt: Preempter,
}

impl StateInner {
    fn new(refresh_preempt: Preempter, scan_preempt: Preempter) -> Self {
        Self {
            current_wps: Default::default(),
            frozen: Default::default(),
            scan_preempt,
            refresh_preempt,
        }
    }
}

struct ControlLoop<'a, Reload, Terminate> {
    cfg_reload: &'a mut Reload,
    terminate: &'a mut Terminate,
    image_scanner: &'a mut ImageScanner,
    pool: &'a sqlx::SqlitePool,
    cfg: &'a Config,
    state: &'a State,
    mon: &'a mut dyn Monitor,
    root: RootData,
}

impl<'a, Reload, Terminate> ControlLoop<'a, Reload, Terminate>
where
    Reload: Stream<Item = Vec<u8>> + Unpin,
    Terminate: Stream<Item = ()> + Unpin,
{
    async fn pickwp(&mut self) -> Result<(), Error> {
        if let Some(state) = self.state.lock().await.as_mut() {
            if state.frozen {
                return Ok(());
            }

            let mut cxn = self.pool.acquire().await.unwrap();
            state.current_wps.clear();
            for monitor in self.mon.idents().await? {
                let ent = match db::pickwp(&mut cxn, self.root.id(), &self.cfg.filter).await? {
                    Some((_, path)) => {
                        let absolute_path = self.root.root(&path);

                        self.mon
                            .set_wallpaper(self.cfg.mode, &monitor, &absolute_path)
                            .await?;

                        tracing::info!(
                            monitor = monitor.as_str(),
                            path = absolute_path.as_str(),
                            "Set wp"
                        );
                        Some(absolute_path)
                    }
                    None => {
                        tracing::info!(monitor = monitor.as_str(), "No wp found for");
                        None
                    }
                };
                state.current_wps.insert(monitor, ent);
            }
        }

        Ok(())
    }

    async fn run(mut self) -> Result<LoopExit, Error> {
        let (mut refresh_preempt, mut refresh) =
            crate::util::preemptible_interval(self.cfg.refresh_interval);
        let (mut rescan_preempt, mut rescan) =
            crate::util::preemptible_interval(self.cfg.rescan_interval);
        refresh_preempt.preempt().await;
        rescan_preempt.preempt().await;

        self.state
            .store(StateInner::new(refresh_preempt, rescan_preempt))
            .await;

        loop {
            tokio::select! {
                Some(new_cfg) = self.cfg_reload.next() => {
                    match Config::from_slice(&new_cfg) {
                        Ok(new_cfg) => {
                            tracing::info!("Reloaded config");
                            return Ok(LoopExit::NewCfg(new_cfg));
                        }
                        Err(e) => {
                            tracing::error!("{}", e);
                        }
                    }
                }

                Some(_) = self.terminate.next() => {
                    return Ok(LoopExit::Terminate);
                }

                Some(_) = rescan.next() => {
                    self.image_scanner.start_scan(self.pool, self.root.clone());
                }

                Some(_) = refresh.next() => {
                    if let Err(e) = self.pickwp().await {
                        tracing::error!("{}", e);
                    }
                }
            }
        }
    }
}

enum LoopExit {
    NewCfg(Config),
    Terminate,
}

fn signal_stream(signals: &[SignalKind]) -> Result<impl Stream<Item = ()>, Error> {
    Ok(stream::select_all(
        signals
            .iter()
            .map(|signo| signal(*signo).context(RegisterSignal))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

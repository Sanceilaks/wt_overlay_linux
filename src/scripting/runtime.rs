use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    latest::{Latest, LatestPublisher},
    scene::TextScene,
    telemetry::TelemetrySnapshot,
};

use super::{ScriptError, ScriptFileWatcher, SteelHudScriptEngine, worker::ReloadingScript};

const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);
const IDLE_CHECK_INTERVAL: Duration = Duration::from_millis(50);

/// Owns the dedicated Steel thread. Dropping it requests shutdown and joins it.
pub struct ScriptWorker {
    shutdown: Arc<AtomicBool>,
    wake: SyncSender<()>,
    join: Option<JoinHandle<()>>,
}

impl ScriptWorker {
    pub fn spawn(
        script_path: PathBuf,
        telemetry: Latest<TelemetrySnapshot>,
        scenes: LatestPublisher<TextScene>,
    ) -> Result<Self, ScriptError> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let (wake, wake_rx) = sync_channel(1);
        let (started_tx, started_rx) = sync_channel(1);

        let join = thread::Builder::new()
            .name("wt-hud-script".into())
            .spawn(move || {
                let source = fs::read_to_string(&script_path).map_err(|error| {
                    ScriptError::io(format!(
                        "failed to read initial script {}: {error}",
                        script_path.display()
                    ))
                });
                let mut script = ReloadingScript::new(SteelHudScriptEngine::new());
                let latest = telemetry.try_snapshot();
                let startup = source.and_then(|source| script.load(&source)).and_then(|()| {
                    let telemetry = latest.as_ref().ok_or_else(|| {
                        ScriptError::evaluate("initial telemetry snapshot is unavailable")
                    })?;
                    let scene = script.evaluate(&telemetry.value)?.clone();
                    scenes.publish(scene).map_err(|error| {
                        ScriptError::evaluate(format!("failed to publish initial scene: {error}"))
                    })?;
                    Ok(())
                });
                let can_run = startup.is_ok();
                let _ = started_tx.send(startup);
                if !can_run {
                    return;
                }

                let mut latest_snapshot = latest.map(|value| value.value);
                let mut watcher = ScriptFileWatcher::new(script_path, RELOAD_DEBOUNCE);
                while !worker_shutdown.load(Ordering::Acquire) {
                    let _ = wake_rx.recv_timeout(IDLE_CHECK_INTERVAL);

                    if let Some(update) = telemetry.take_update() {
                        latest_snapshot = Some(Arc::clone(&update.value));
                        match script.evaluate(&update.value) {
                            Ok(scene) => publish_scene(&scenes, scene.clone()),
                            Err(error) => tracing::warn!(%error, "HUD script evaluation failed"),
                        }
                    }

                    match watcher.poll(Instant::now()) {
                        Ok(Some(source)) => match script.load(&source) {
                            Ok(()) => {
                                if let Some(snapshot) = latest_snapshot.as_deref() {
                                    match script.evaluate(snapshot) {
                                        Ok(scene) => {
                                            publish_scene(&scenes, scene.clone());
                                            tracing::info!("HUD script reload succeeded");
                                        }
                                        Err(error) => tracing::warn!(
                                            %error,
                                            "HUD script reload evaluation failed; keeping last good program and scene"
                                        ),
                                    }
                                }
                            }
                            Err(error) => tracing::warn!(
                                %error,
                                "HUD script reload failed; keeping last good program and scene"
                            ),
                        },
                        Ok(None) => {}
                        Err(error) => tracing::warn!(%error, "HUD script watcher failed"),
                    }
                }
            })
            .map_err(|error| ScriptError::io(format!("failed to spawn script worker: {error}")))?;

        match started_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                shutdown,
                wake,
                join: Some(join),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(error) => {
                let _ = join.join();
                Err(ScriptError::io(format!(
                    "script worker stopped during startup: {error}"
                )))
            }
        }
    }

    pub fn shutdown(mut self) -> thread::Result<()> {
        self.request_shutdown();
        self.join.take().expect("worker join handle exists").join()
    }

    fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
    }
}

impl Drop for ScriptWorker {
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn publish_scene(publisher: &LatestPublisher<TextScene>, scene: TextScene) {
    if let Err(error) = publisher.publish(scene) {
        tracing::error!(%error, "scene revision counter exhausted");
    }
}

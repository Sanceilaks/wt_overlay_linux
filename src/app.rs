use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use calloop::ping::{Ping, PingSource, make_ping};
use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};

use crate::{
    config::Config,
    latest::Latest,
    layout::{LayoutEngine, LogicalSize, ShapedTextSize, TextMeasurer},
    overlay::{Overlay, OverlayRenderer},
    render::{RenderText, WgpuTextRenderer, bundled_font_system},
    scene::{FontWeight, Rgba, TextNode, TextScene},
    scripting::{HudScriptEngine, ReloadingScript, ScriptFileWatcher, SteelHudScriptEngine},
    telemetry::{RawTelemetry, TelemetrySnapshot, TelemetryStatus, TelemetryWorker, WorkerConfig},
};

pub fn run(config_path: &str) -> Result<()> {
    let config = Config::load(config_path)?;
    let source = fs::read_to_string(&config.script)
        .with_context(|| format!("failed to read HUD script {}", config.script.display()))?;
    let initial_telemetry = TelemetrySnapshot::with_status(
        0,
        Instant::now(),
        TelemetryStatus::Disconnected,
        RawTelemetry::default(),
    );
    let telemetry_latest = Latest::new(initial_telemetry);
    let telemetry_publisher = telemetry_latest.publisher();
    let scene_latest = Latest::new(TextScene::default());
    let scene_publisher = scene_latest.publisher();
    let (scene_ping, scene_wake) = make_ping()?;

    let script_worker = ScriptWorker::spawn(
        source,
        config.script.clone(),
        telemetry_latest,
        scene_publisher,
        scene_ping,
    )?;
    let telemetry_worker = TelemetryWorker::spawn(
        WorkerConfig {
            poll_interval: config.poll_interval,
            request_timeout: config.request_timeout,
            ..WorkerConfig::default()
        },
        move |snapshot: Arc<TelemetrySnapshot>| {
            let _ = telemetry_publisher.publish((*snapshot).clone());
        },
    )?;

    let result = run_overlay(&config, scene_latest, scene_wake);
    drop(telemetry_worker);
    drop(script_worker);
    result
}

fn run_overlay(config: &Config, scenes: Latest<TextScene>, scene_wake: PingSource) -> Result<()> {
    let overlay =
        Overlay::new(&config.output).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut renderer = WgpuTextRenderer::new(overlay.connection(), overlay.surface())
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let (width, height, scale) = overlay.size_and_scale();
    renderer.resize(width, height, scale);
    let renderer = SceneRenderer::new(renderer, scenes, width, height)?;
    overlay
        .run(renderer, scene_wake)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

struct ScriptWorker {
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ScriptWorker {
    fn spawn(
        initial_source: String,
        script_path: std::path::PathBuf,
        telemetry: Latest<TelemetrySnapshot>,
        scenes: crate::latest::LatestPublisher<TextScene>,
        ping: Ping,
    ) -> Result<Self> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let startup_path = script_path.clone();
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("hud-script".into())
            .spawn(move || {
                let mut engine = SteelHudScriptEngine::new();
                if let Err(error) = engine.load(&initial_source) {
                    let _ = startup_tx.send(Err(error.to_string()));
                    return;
                }
                let _ = startup_tx.send(Ok(()));
                let mut script = ReloadingScript::new(engine);
                let mut watcher = ScriptFileWatcher::new(script_path.clone(), Duration::from_millis(150));
                let mut current = telemetry.try_snapshot().map(|value| value.value);
                if let Some(snapshot) = current.as_deref() {
                    publish_scene(&mut script, snapshot, &scenes, &ping);
                }

                while !worker_shutdown.load(Ordering::Acquire) {
                    if let Some(update) = telemetry.take_update() {
                        current = Some(update.value);
                        if let Some(snapshot) = current.as_deref() {
                            publish_scene(&mut script, snapshot, &scenes, &ping);
                        }
                    }
                    match watcher.poll(Instant::now()) {
                        Ok(Some(source)) => match script.load(&source) {
                            Ok(()) => {
                                tracing::info!(path = %script_path.display(), "HUD script reloaded");
                                if let Some(snapshot) = current.as_deref() {
                                    publish_scene(&mut script, snapshot, &scenes, &ping);
                                }
                            }
                            Err(error) => tracing::error!(path = %script_path.display(), %error, "HUD script reload rejected; keeping last valid script and scene"),
                        },
                        Ok(None) => {}
                        Err(error) => tracing::warn!(path = %script_path.display(), %error, "HUD script watch failed"),
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            })
            .context("failed to spawn Steel script worker")?;
        match startup_rx
            .recv()
            .context("Steel script worker exited during startup")?
        {
            Ok(()) => {}
            Err(error) => {
                let _ = thread.join();
                anyhow::bail!(
                    "initial Steel compilation failed for {}: {error}",
                    startup_path.display()
                );
            }
        }
        Ok(Self {
            shutdown,
            thread: Some(thread),
        })
    }
}

impl Drop for ScriptWorker {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn publish_scene(
    script: &mut ReloadingScript<SteelHudScriptEngine>,
    telemetry: &TelemetrySnapshot,
    scenes: &crate::latest::LatestPublisher<TextScene>,
    ping: &Ping,
) {
    match script.evaluate(telemetry) {
        Ok(scene) => {
            let _ = scenes.publish(scene.clone());
            ping.ping();
        }
        Err(error) => {
            tracing::error!(%error, "HUD script evaluation failed; keeping last valid scene")
        }
    }
}

struct SceneRenderer {
    gpu: WgpuTextRenderer,
    scenes: Latest<TextScene>,
    current: TextScene,
    layout: LayoutEngine,
    measurer: GlyphMeasurer,
    logical_size: LogicalSize,
    started: Instant,
    next_animation: Option<Duration>,
}

impl SceneRenderer {
    fn new(
        gpu: WgpuTextRenderer,
        scenes: Latest<TextScene>,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let current = scenes
            .try_snapshot()
            .map_or_else(TextScene::default, |value| (*value.value).clone());
        Ok(Self {
            gpu,
            scenes,
            current,
            layout: LayoutEngine::default(),
            measurer: GlyphMeasurer::new(),
            logical_size: LogicalSize::new(width as f32, height as f32),
            started: Instant::now(),
            next_animation: None,
        })
    }

    fn prepare_scene(&mut self) -> Result<()> {
        if let Some(scene) = self.scenes.take_update() {
            self.current = (*scene.value).clone();
        }
        let positioned = self
            .layout
            .layout(
                &self.current,
                self.logical_size,
                self.started.elapsed(),
                &self.measurer,
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        self.next_animation = positioned.next_animation_in;
        self.gpu.set_items(
            positioned
                .nodes
                .into_iter()
                .filter(|node| node.visible)
                .map(|positioned| {
                    let style = positioned.node.style;
                    RenderText {
                        id: positioned.node.id,
                        text: positioned.node.text,
                        left: positioned.position.x,
                        top: positioned.position.y,
                        font_size: style.font_size,
                        bold: style.weight == FontWeight::Bold,
                        color: color_bytes(style.foreground),
                        shadow: style.shadow.map(color_bytes),
                    }
                })
                .collect(),
        );
        Ok(())
    }
}

impl OverlayRenderer for SceneRenderer {
    fn resize(&mut self, width: u32, height: u32, scale: i32) {
        self.logical_size = LogicalSize::new(width as f32, height as f32);
        self.gpu.resize(width, height, scale);
    }

    fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.prepare_scene()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        self.gpu.render()
    }

    fn next_animation_in(&self) -> Option<Duration> {
        self.next_animation
    }
}

struct GlyphMeasurer {
    fonts: std::sync::Mutex<FontSystem>,
}

impl GlyphMeasurer {
    fn new() -> Self {
        Self {
            fonts: std::sync::Mutex::new(bundled_font_system()),
        }
    }
}

impl TextMeasurer for GlyphMeasurer {
    type Error = String;

    fn measure(&self, node: &TextNode) -> Result<ShapedTextSize, Self::Error> {
        let mut fonts = self
            .fonts
            .lock()
            .map_err(|_| "font system lock was poisoned".to_owned())?;
        let line_height = node.style.font_size * 1.2;
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(node.style.font_size, line_height));
        let weight = match node.style.weight {
            FontWeight::Normal => Weight::NORMAL,
            FontWeight::Bold => Weight::BOLD,
        };
        buffer.set_text(
            &node.text,
            &Attrs::new()
                .family(Family::Name("Noto Sans"))
                .weight(weight),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        let mut width = 0.0_f32;
        let mut height = 0.0_f32;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height += run.line_height;
        }
        Ok(ShapedTextSize::new(width, height.max(line_height)))
    }
}

fn color_bytes(color: Rgba) -> [u8; 4] {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        byte(color.red),
        byte(color.green),
        byte(color.blue),
        byte(color.alpha),
    ]
}

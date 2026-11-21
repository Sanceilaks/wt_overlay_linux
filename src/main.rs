pub mod app;
pub mod args;
pub mod config;
pub mod latest;
pub mod layout;
pub mod overlay;
pub mod redraw;
pub mod render;
pub mod scene;
pub mod scripting;
pub mod telemetry;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wt_overlay_linux=info".into()),
        )
        .init();
    let cli = args::parse_args();
    let config_path = cli.hud_scheme_path;
    if let Err(error) = app::run(&config_path) {
        tracing::error!(%error, "HUD terminated");
        std::process::exit(1);
    }
}

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Path to your HUD script
    #[arg(default_value = "hud.scm")]
    pub hud_scheme_path: String,
}

pub fn parse_args() -> Cli {
    Cli::parse()
}

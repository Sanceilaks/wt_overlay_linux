use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;

const MAX_INTERVAL_MS: u64 = 60_000;

/// Operational settings which require an application restart when changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub output: String,
    pub script: PathBuf,
    pub poll_interval: Duration,
    pub request_timeout: Duration,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    output: String,
    script: PathBuf,
    poll_interval_ms: u64,
    request_timeout_ms: u64,
}

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    Invalid(String),
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml(path, &source)
    }

    pub fn from_toml(path: impl AsRef<Path>, source: &str) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let parsed: ConfigFile = toml::from_str(source).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

        let output = parsed.output.trim();
        if output.is_empty() {
            return Err(ConfigError::Invalid(
                "`output` must be a non-empty exact Wayland output name".into(),
            ));
        }
        if parsed.script.as_os_str().is_empty() {
            return Err(ConfigError::Invalid("`script` must not be empty".into()));
        }
        validate_interval("poll_interval_ms", parsed.poll_interval_ms)?;
        validate_interval("request_timeout_ms", parsed.request_timeout_ms)?;

        let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let script = if parsed.script.is_absolute() {
            parsed.script
        } else {
            config_dir.join(parsed.script)
        };

        Ok(Self {
            output: output.to_owned(),
            script,
            poll_interval: Duration::from_millis(parsed.poll_interval_ms),
            request_timeout: Duration::from_millis(parsed.request_timeout_ms),
        })
    }
}

fn validate_interval(name: &str, value: u64) -> Result<(), ConfigError> {
    if !(1..=MAX_INTERVAL_MS).contains(&value) {
        return Err(ConfigError::Invalid(format!(
            "`{name}` must be between 1 and {MAX_INTERVAL_MS} milliseconds"
        )));
    }
    Ok(())
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    f,
                    "failed to read configuration {}: {source}",
                    path.display()
                )
            }
            Self::Parse { path, source } => {
                write!(
                    f,
                    "failed to parse configuration {}: {source}",
                    path.display()
                )
            }
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Invalid(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
        output = "DP-1"
        script = "scripts/hud.scm"
        poll_interval_ms = 100
        request_timeout_ms = 300
    "#;

    #[test]
    fn loads_and_resolves_relative_script_path() {
        let config = Config::from_toml("/tmp/wt-hud/config.toml", VALID).unwrap();
        assert_eq!(config.output, "DP-1");
        assert_eq!(config.script, Path::new("/tmp/wt-hud/scripts/hud.scm"));
        assert_eq!(config.poll_interval, Duration::from_millis(100));
        assert_eq!(config.request_timeout, Duration::from_millis(300));
    }

    #[test]
    fn preserves_absolute_script_path() {
        let source = VALID.replace("scripts/hud.scm", "/etc/wt-hud/hud.scm");
        let config = Config::from_toml("config.toml", &source).unwrap();
        assert_eq!(config.script, Path::new("/etc/wt-hud/hud.scm"));
    }

    #[test]
    fn rejects_missing_output() {
        let source = VALID.replace("output = \"DP-1\"\n", "");
        assert!(matches!(
            Config::from_toml("config.toml", &source),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn rejects_empty_output_and_invalid_intervals() {
        let empty_output = VALID.replace("DP-1", "   ");
        assert!(matches!(
            Config::from_toml("config.toml", &empty_output),
            Err(ConfigError::Invalid(_))
        ));

        for (name, value) in [("poll_interval_ms", 0), ("request_timeout_ms", 60_001)] {
            let source = VALID.replace(
                &format!(
                    "{name} = {}",
                    if name == "poll_interval_ms" { 100 } else { 300 }
                ),
                &format!("{name} = {value}"),
            );
            assert!(matches!(
                Config::from_toml("config.toml", &source),
                Err(ConfigError::Invalid(_))
            ));
        }
    }
}

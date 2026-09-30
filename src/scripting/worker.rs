use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use crate::{scene::TextScene, telemetry::TelemetrySnapshot};

use super::{HudScriptEngine, ScriptError};

/// Transactional reload state used by the script worker.
///
/// Neither a bad replacement program nor an evaluation failure clears the last
/// scene which the main thread can safely continue to display.
pub struct ReloadingScript<E> {
    engine: E,
    last_scene: Option<TextScene>,
}

impl<E: HudScriptEngine> ReloadingScript<E> {
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            last_scene: None,
        }
    }

    pub fn load(&mut self, source: &str) -> Result<(), ScriptError> {
        self.engine.load(source)
    }

    pub fn evaluate(&mut self, telemetry: &TelemetrySnapshot) -> Result<&TextScene, ScriptError> {
        let scene = self.engine.evaluate(telemetry)?;
        self.last_scene = Some(scene);
        Ok(self.last_scene.as_ref().expect("scene was just assigned"))
    }

    pub fn last_scene(&self) -> Option<&TextScene> {
        self.last_scene.as_ref()
    }

    pub fn into_engine(self) -> E {
        self.engine
    }
}

/// Metadata-based hot-reload helper. Call `poll` from the script worker's
/// timeout loop; it never belongs on the Wayland/render thread.
pub struct ScriptFileWatcher {
    path: PathBuf,
    debounce: Duration,
    observed: Option<FileStamp>,
    pending_since: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl ScriptFileWatcher {
    pub fn new(path: impl Into<PathBuf>, debounce: Duration) -> Self {
        Self {
            path: path.into(),
            debounce,
            observed: None,
            pending_since: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns complete source only after file metadata has remained unchanged
    /// for the debounce period.
    pub fn poll(&mut self, now: Instant) -> Result<Option<String>, ScriptError> {
        let metadata = fs::metadata(&self.path).map_err(|error| {
            ScriptError::io(format!("failed to stat {}: {error}", self.path.display()))
        })?;
        let stamp = FileStamp {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        };

        if self.observed != Some(stamp) {
            self.observed = Some(stamp);
            self.pending_since = Some(now);
            return Ok(None);
        }

        let Some(since) = self.pending_since else {
            return Ok(None);
        };
        if now.saturating_duration_since(since) < self.debounce {
            return Ok(None);
        }

        let source = fs::read_to_string(&self.path).map_err(|error| {
            ScriptError::io(format!("failed to read {}: {error}", self.path.display()))
        })?;
        self.pending_since = None;
        Ok(Some(source))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn watcher_debounces_and_reads_the_complete_file() {
        let path = std::env::temp_dir().join(format!(
            "wt-overlay-script-watcher-{}-{}.scm",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        fs::write(&path, "(define x 1)").unwrap();

        let start = Instant::now();
        let mut watcher = ScriptFileWatcher::new(&path, Duration::from_millis(50));
        assert_eq!(watcher.poll(start).unwrap(), None);
        assert_eq!(
            watcher.poll(start + Duration::from_millis(49)).unwrap(),
            None
        );
        assert_eq!(
            watcher.poll(start + Duration::from_millis(50)).unwrap(),
            Some("(define x 1)".into())
        );
        assert_eq!(watcher.poll(start + Duration::from_secs(1)).unwrap(), None);

        fs::remove_file(path).unwrap();
    }
}

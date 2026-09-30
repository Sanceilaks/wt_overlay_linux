use std::{error::Error, fmt};

use crate::{scene::TextScene, telemetry::TelemetrySnapshot};

/// Keeps interpreter details confined to the script worker.
pub trait HudScriptEngine {
    /// Compiles and activates `source`. On error the prior program remains active.
    fn load(&mut self, source: &str) -> Result<(), ScriptError>;

    fn evaluate(&mut self, telemetry: &TelemetrySnapshot) -> Result<TextScene, ScriptError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptError {
    pub phase: ScriptPhase,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptPhase {
    Compile,
    Evaluate,
    Validate,
    Io,
}

impl ScriptError {
    pub fn compile(message: impl Into<String>) -> Self {
        Self::new(ScriptPhase::Compile, message)
    }

    pub fn evaluate(message: impl Into<String>) -> Self {
        Self::new(ScriptPhase::Evaluate, message)
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ScriptPhase::Validate, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ScriptPhase::Io, message)
    }

    fn new(phase: ScriptPhase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
        }
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} error: {}", self.phase, self.message)
    }
}

impl Error for ScriptError {}

use std::time::Instant;

use serde_json::{Map, Value};

/// The connection and data-validity state reported to HUD scripts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryStatus {
    Disconnected,
    Invalid,
    Hangar,
    Active,
    Stale,
}

/// Unmodified objects returned by War Thunder's telemetry endpoints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawTelemetry {
    pub state: Map<String, Value>,
    pub indicators: Map<String, Value>,
}

impl RawTelemetry {
    pub fn get(&self, endpoint: RawEndpoint, key: &str) -> Option<&Value> {
        match endpoint {
            RawEndpoint::State => self.state.get(key),
            RawEndpoint::Indicators => self.indicators.get(key),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawEndpoint {
    State,
    Indicators,
}

/// A stable view over the aircraft-dependent raw telemetry maps.
#[derive(Clone, Debug, PartialEq)]
pub struct TelemetrySnapshot {
    pub revision: u64,
    pub received_at: Instant,
    pub status: TelemetryStatus,
    pub ias_kmh: Option<f64>,
    pub tas_kmh: Option<f64>,
    pub aoa_deg: Option<f64>,
    pub overload_g: Option<f64>,
    pub altitude_m: Option<f64>,
    pub vertical_speed_ms: Option<f64>,
    pub raw: RawTelemetry,
}

impl TelemetrySnapshot {
    pub(crate) fn with_status(
        revision: u64,
        received_at: Instant,
        status: TelemetryStatus,
        raw: RawTelemetry,
    ) -> Self {
        Self {
            revision,
            received_at,
            status,
            ias_kmh: None,
            tas_kmh: None,
            aoa_deg: None,
            overload_g: None,
            altitude_m: None,
            vertical_speed_ms: None,
            raw,
        }
    }
}

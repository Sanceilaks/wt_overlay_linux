use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use crate::telemetry::{TelemetrySnapshot, TelemetryStatus};

pub const MAX_STATE_DURATION: Duration = Duration::from_secs(30);
const MAX_SAMPLE_GAP: Duration = Duration::from_secs(2);
const MAX_UNUSED_AGE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
pub(super) enum EdgeDirection {
    Rising,
    Falling,
}

#[derive(Clone, Default)]
pub(super) struct SharedStatefulValues(Arc<RwLock<StatefulValues>>);

impl SharedStatefulValues {
    pub fn begin_evaluation(&self, snapshot: &TelemetrySnapshot) {
        self.write().begin_evaluation(snapshot);
    }

    pub fn edge(&self, key: &str, value: bool, direction: EdgeDirection) -> bool {
        self.write().edge(key, value, direction)
    }

    pub fn hold(&self, key: &str, value: bool, duration: Duration) -> bool {
        self.write().hold(key, value, duration)
    }

    pub fn debounce(&self, key: &str, value: bool, duration: Duration) -> bool {
        self.write().debounce(key, value, duration)
    }

    pub fn ema(&self, key: &str, value: f64, alpha: f64) -> Option<f64> {
        self.write().ema(key, value, alpha)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, StatefulValues> {
        self.0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Clone, Copy)]
struct Evaluation {
    revision: u64,
    received_at: Instant,
}

#[derive(Clone, Copy)]
struct EdgeState {
    previous: bool,
    initialized: bool,
    output: bool,
    revision: u64,
    used_at: Instant,
}

#[derive(Clone, Copy)]
struct HoldState {
    until: Option<Instant>,
    output: bool,
    revision: u64,
    used_at: Instant,
}

#[derive(Clone, Copy)]
struct DebounceState {
    true_since: Option<Instant>,
    output: bool,
    revision: u64,
    used_at: Instant,
}

#[derive(Clone, Copy)]
struct EmaState {
    smoothed: f64,
    revision: u64,
    used_at: Instant,
}

#[derive(Default)]
struct StatefulValues {
    current: Option<Evaluation>,
    edges: HashMap<String, EdgeState>,
    holds: HashMap<String, HoldState>,
    debounces: HashMap<String, DebounceState>,
    emas: HashMap<String, EmaState>,
}

impl StatefulValues {
    fn begin_evaluation(&mut self, snapshot: &TelemetrySnapshot) {
        if snapshot.status != TelemetryStatus::Active {
            self.clear();
            return;
        }

        if self.current.is_some_and(|previous| {
            snapshot
                .received_at
                .checked_duration_since(previous.received_at)
                .is_none_or(|gap| gap > MAX_SAMPLE_GAP)
        }) {
            self.clear();
        }

        self.current = Some(Evaluation {
            revision: snapshot.revision,
            received_at: snapshot.received_at,
        });
        self.prune(snapshot.received_at);
    }

    fn edge(&mut self, key: &str, value: bool, direction: EdgeDirection) -> bool {
        let Some(current) = self.current else {
            return false;
        };
        let state = self.edges.entry(key.to_owned()).or_insert(EdgeState {
            previous: value,
            initialized: false,
            output: false,
            revision: current.revision,
            used_at: current.received_at,
        });
        if state.revision == current.revision && state.initialized {
            return state.output;
        }

        let output = state.initialized
            && match direction {
                EdgeDirection::Rising => !state.previous && value,
                EdgeDirection::Falling => state.previous && !value,
            };
        *state = EdgeState {
            previous: value,
            initialized: true,
            output,
            revision: current.revision,
            used_at: current.received_at,
        };
        output
    }

    fn hold(&mut self, key: &str, value: bool, duration: Duration) -> bool {
        let Some(current) = self.current else {
            return false;
        };
        let state = self.holds.entry(key.to_owned()).or_insert(HoldState {
            until: None,
            output: false,
            revision: u64::MAX,
            used_at: current.received_at,
        });
        if state.revision == current.revision {
            return state.output;
        }

        if value {
            state.until = current.received_at.checked_add(duration);
        }
        state.output = value
            || state
                .until
                .is_some_and(|deadline| current.received_at < deadline);
        state.revision = current.revision;
        state.used_at = current.received_at;
        state.output
    }

    fn debounce(&mut self, key: &str, value: bool, duration: Duration) -> bool {
        let Some(current) = self.current else {
            return false;
        };
        let state = self
            .debounces
            .entry(key.to_owned())
            .or_insert(DebounceState {
                true_since: None,
                output: false,
                revision: u64::MAX,
                used_at: current.received_at,
            });
        if state.revision == current.revision {
            return state.output;
        }

        if value {
            let since = *state.true_since.get_or_insert(current.received_at);
            state.output = current
                .received_at
                .checked_duration_since(since)
                .is_some_and(|elapsed| elapsed >= duration);
        } else {
            state.true_since = None;
            state.output = false;
        }
        state.revision = current.revision;
        state.used_at = current.received_at;
        state.output
    }

    fn ema(&mut self, key: &str, value: f64, alpha: f64) -> Option<f64> {
        let current = self.current?;
        let state = self.emas.entry(key.to_owned()).or_insert(EmaState {
            smoothed: value,
            revision: current.revision,
            used_at: current.received_at,
        });
        if state.revision != current.revision {
            state.smoothed = alpha * value + (1.0 - alpha) * state.smoothed;
            state.revision = current.revision;
            state.used_at = current.received_at;
        }
        Some(state.smoothed)
    }

    fn prune(&mut self, now: Instant) {
        self.edges
            .retain(|_, state| recently_used(now, state.used_at));
        self.holds
            .retain(|_, state| recently_used(now, state.used_at));
        self.debounces
            .retain(|_, state| recently_used(now, state.used_at));
        self.emas
            .retain(|_, state| recently_used(now, state.used_at));
    }

    fn clear(&mut self) {
        self.current = None;
        self.edges.clear();
        self.holds.clear();
        self.debounces.clear();
        self.emas.clear();
    }
}

fn recently_used(now: Instant, used_at: Instant) -> bool {
    now.checked_duration_since(used_at)
        .is_some_and(|age| age <= MAX_UNUSED_AGE)
}

#[cfg(test)]
mod tests {
    use crate::telemetry::RawTelemetry;

    use super::*;

    fn snapshot(revision: u64, received_at: Instant) -> TelemetrySnapshot {
        TelemetrySnapshot {
            revision,
            received_at,
            status: TelemetryStatus::Active,
            ias_kmh: None,
            tas_kmh: None,
            aoa_deg: None,
            overload_g: None,
            altitude_m: None,
            vertical_speed_ms: None,
            raw: RawTelemetry::default(),
        }
    }

    #[test]
    fn edge_emits_for_exactly_one_evaluation() {
        let values = SharedStatefulValues::default();
        let start = Instant::now();
        values.begin_evaluation(&snapshot(1, start));
        assert!(!values.edge("warning", false, EdgeDirection::Rising));
        values.begin_evaluation(&snapshot(2, start + Duration::from_millis(100)));
        assert!(values.edge("warning", true, EdgeDirection::Rising));
        assert!(values.edge("warning", true, EdgeDirection::Rising));
        values.begin_evaluation(&snapshot(3, start + Duration::from_millis(200)));
        assert!(!values.edge("warning", true, EdgeDirection::Rising));
    }

    #[test]
    fn hold_extends_a_short_pulse() {
        let values = SharedStatefulValues::default();
        let start = Instant::now();
        values.begin_evaluation(&snapshot(1, start));
        assert!(values.hold("warning", true, Duration::from_secs(1)));
        values.begin_evaluation(&snapshot(2, start + Duration::from_millis(900)));
        assert!(values.hold("warning", false, Duration::from_secs(1)));
        values.begin_evaluation(&snapshot(3, start + Duration::from_secs(1)));
        assert!(!values.hold("warning", false, Duration::from_secs(1)));
    }

    #[test]
    fn debounce_requires_continuous_true_time() {
        let values = SharedStatefulValues::default();
        let start = Instant::now();
        values.begin_evaluation(&snapshot(1, start));
        assert!(!values.debounce("warning", true, Duration::from_secs(1)));
        values.begin_evaluation(&snapshot(2, start + Duration::from_millis(500)));
        assert!(!values.debounce("warning", false, Duration::from_secs(1)));
        values.begin_evaluation(&snapshot(3, start + Duration::from_secs(1)));
        assert!(!values.debounce("warning", true, Duration::from_secs(1)));
        values.begin_evaluation(&snapshot(4, start + Duration::from_secs(2)));
        assert!(values.debounce("warning", true, Duration::from_secs(1)));
    }

    #[test]
    fn ema_keeps_one_smoothed_value_per_key() {
        let values = SharedStatefulValues::default();
        let start = Instant::now();
        values.begin_evaluation(&snapshot(1, start));
        assert_eq!(values.ema("aoa", 10.0, 0.25), Some(10.0));
        values.begin_evaluation(&snapshot(2, start + Duration::from_millis(100)));
        assert_eq!(values.ema("aoa", 14.0, 0.25), Some(11.0));
    }
}

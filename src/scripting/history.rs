use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use crate::telemetry::{TelemetrySnapshot, TelemetryStatus};

pub const MAX_HISTORY: Duration = Duration::from_secs(30);
const MAX_SAMPLE_GAP: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
pub(super) struct SharedMetricHistory(Arc<RwLock<MetricHistory>>);

impl SharedMetricHistory {
    pub fn begin_evaluation(&self, snapshot: &TelemetrySnapshot) {
        self.write().begin_evaluation(snapshot);
    }

    pub fn series(&self, name: &str, value: Option<f64>, window: Duration) -> Vec<(Duration, f64)> {
        self.write().series(name, value, window)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, MetricHistory> {
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
struct MetricSample {
    revision: u64,
    received_at: Instant,
    value: f64,
}

#[derive(Default)]
struct MetricHistory {
    current: Option<Evaluation>,
    series: HashMap<String, VecDeque<MetricSample>>,
}

impl MetricHistory {
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
            self.series.clear();
        }

        self.current = Some(Evaluation {
            revision: snapshot.revision,
            received_at: snapshot.received_at,
        });
        self.prune(snapshot.received_at);
    }

    fn series(&mut self, name: &str, value: Option<f64>, window: Duration) -> Vec<(Duration, f64)> {
        let Some(current) = self.current else {
            return Vec::new();
        };
        let samples = self.series.entry(name.to_owned()).or_default();
        if let Some(value) = value
            && samples
                .back()
                .is_none_or(|sample| sample.revision != current.revision)
        {
            samples.push_back(MetricSample {
                revision: current.revision,
                received_at: current.received_at,
                value,
            });
        }

        samples
            .iter()
            .filter_map(|sample| {
                current
                    .received_at
                    .checked_duration_since(sample.received_at)
                    .filter(|age| *age <= window)
                    .map(|age| (age, sample.value))
            })
            .collect()
    }

    fn prune(&mut self, now: Instant) {
        for samples in self.series.values_mut() {
            while samples.front().is_some_and(|sample| {
                now.checked_duration_since(sample.received_at)
                    .is_some_and(|age| age > MAX_HISTORY)
            }) {
                samples.pop_front();
            }
        }
        self.series.retain(|_, samples| !samples.is_empty());
    }

    fn clear(&mut self) {
        self.current = None;
        self.series.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::telemetry::RawTelemetry;

    use super::*;

    fn snapshot(revision: u64, received_at: Instant) -> TelemetrySnapshot {
        TelemetrySnapshot {
            revision,
            received_at,
            status: TelemetryStatus::Active,
            ias_kmh: Some(300.0),
            tas_kmh: None,
            aoa_deg: None,
            overload_g: None,
            altitude_m: None,
            vertical_speed_ms: None,
            raw: RawTelemetry::default(),
        }
    }

    #[test]
    fn stores_only_named_values_and_ignores_duplicate_evaluations() {
        let history = SharedMetricHistory::default();
        let start = Instant::now();
        history.begin_evaluation(&snapshot(1, start));
        history.series("ias", Some(300.0), MAX_HISTORY);
        history.begin_evaluation(&snapshot(2, start + Duration::from_secs(1)));
        history.series("ias", Some(280.0), MAX_HISTORY);
        history.series("ias", Some(123.0), MAX_HISTORY);

        let samples = history.series("ias", None, Duration::from_millis(1100));
        assert_eq!(
            samples,
            vec![(Duration::from_secs(1), 300.0), (Duration::ZERO, 280.0)]
        );
        assert!(history.series("unused", None, MAX_HISTORY).is_empty());
    }

    #[test]
    fn discontinuity_clears_all_named_series() {
        let history = SharedMetricHistory::default();
        let start = Instant::now();
        history.begin_evaluation(&snapshot(1, start));
        history.series("ias", Some(300.0), MAX_HISTORY);
        history.begin_evaluation(&snapshot(
            2,
            start + MAX_SAMPLE_GAP + Duration::from_millis(1),
        ));

        assert!(history.series("ias", None, MAX_HISTORY).is_empty());

        history.series("ias", Some(280.0), MAX_HISTORY);
        let mut disconnected = snapshot(3, start + Duration::from_secs(3));
        disconnected.status = TelemetryStatus::Disconnected;
        history.begin_evaluation(&disconnected);
        assert!(history.series("ias", None, MAX_HISTORY).is_empty());
    }
}

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use super::client::{ClientError, TelemetryClient, UreqTelemetryClient};
use super::model::{RawTelemetry, TelemetrySnapshot, TelemetryStatus};
use super::normalize::{ParseError, parse_and_normalize_with_map_info};

pub trait TelemetryPublisher: Send + Sync + 'static {
    fn publish(&self, snapshot: Arc<TelemetrySnapshot>);
}

impl<F> TelemetryPublisher for F
where
    F: Fn(Arc<TelemetrySnapshot>) + Send + Sync + 'static,
{
    fn publish(&self, snapshot: Arc<TelemetrySnapshot>) {
        self(snapshot);
    }
}

#[derive(Clone, Debug)]
pub struct WorkerConfig {
    pub poll_interval: Duration,
    pub request_timeout: Duration,
    pub stale_after: Duration,
    pub max_backoff: Duration,
    pub repeated_error_log_interval: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(100),
            request_timeout: Duration::from_millis(300),
            stale_after: Duration::from_secs(2),
            max_backoff: Duration::from_secs(5),
            repeated_error_log_interval: Duration::from_secs(10),
        }
    }
}

impl WorkerConfig {
    pub fn validate(&self) -> Result<(), SpawnError> {
        if self.poll_interval.is_zero() {
            return Err(SpawnError::InvalidConfig("poll interval must be positive"));
        }
        if self.request_timeout.is_zero() {
            return Err(SpawnError::InvalidConfig(
                "request timeout must be positive",
            ));
        }
        if self.stale_after.is_zero() {
            return Err(SpawnError::InvalidConfig(
                "stale threshold must be positive",
            ));
        }
        if self.max_backoff < self.poll_interval {
            return Err(SpawnError::InvalidConfig(
                "maximum backoff cannot be shorter than poll interval",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum SpawnError {
    InvalidConfig(&'static str),
    Client(ClientError),
    Thread(std::io::Error),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::Client(error) => write!(formatter, "failed to create telemetry client: {error}"),
            Self::Thread(error) => write!(formatter, "failed to spawn telemetry worker: {error}"),
        }
    }
}

impl std::error::Error for SpawnError {}

pub struct TelemetryWorker {
    shutdown: Arc<AtomicBool>,
    wake: SyncSender<()>,
    thread: Option<JoinHandle<()>>,
}

impl TelemetryWorker {
    pub fn spawn(
        config: WorkerConfig,
        publisher: impl TelemetryPublisher,
    ) -> Result<Self, SpawnError> {
        config.validate()?;
        let client = UreqTelemetryClient::new("http://127.0.0.1:8111", config.request_timeout)
            .map_err(SpawnError::Client)?;
        Self::spawn_with_client(config, client, publisher)
    }

    pub fn spawn_with_client<C, P>(
        config: WorkerConfig,
        client: C,
        publisher: P,
    ) -> Result<Self, SpawnError>
    where
        C: TelemetryClient,
        P: TelemetryPublisher,
    {
        config.validate()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let (wake, wake_rx) = sync_channel(1);
        let worker_shutdown = Arc::clone(&shutdown);
        let thread = thread::Builder::new()
            .name("telemetry".to_owned())
            .spawn(move || run(client, publisher, config, worker_shutdown, wake_rx))
            .map_err(SpawnError::Thread)?;

        Ok(Self {
            shutdown,
            wake,
            thread: Some(thread),
        })
    }

    pub fn shutdown(mut self) -> thread::Result<()> {
        self.signal_shutdown();
        self.thread.take().expect("worker thread is present").join()
    }

    fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
    }
}

impl Drop for TelemetryWorker {
    fn drop(&mut self) {
        self.signal_shutdown();
        if let Some(thread) = self.thread.take()
            && thread.thread().id() != thread::current().id()
        {
            let _ = thread.join();
        }
    }
}

fn run<C, P>(
    client: C,
    publisher: P,
    config: WorkerConfig,
    shutdown: Arc<AtomicBool>,
    wake: Receiver<()>,
) where
    C: TelemetryClient,
    P: TelemetryPublisher,
{
    let mut state = WorkerState::new(config.poll_interval, config.repeated_error_log_interval);

    while !shutdown.load(Ordering::Acquire) {
        let cycle_started = Instant::now();
        let result = poll(&client, state.next_revision());
        let now = Instant::now();
        let (snapshot, succeeded, diagnostic) = state.classify(result, now, &config);
        state.log_transition(snapshot.status, diagnostic.as_deref(), now);
        publisher.publish(Arc::new(snapshot));

        let delay = if succeeded {
            state.backoff.reset();
            config.poll_interval
        } else {
            state.backoff.next_delay()
        };
        let remaining = delay.saturating_sub(cycle_started.elapsed());
        if !remaining.is_zero() {
            let _ = wake.recv_timeout(remaining);
        }
    }
    debug!("telemetry worker stopped");
}

fn poll(client: &impl TelemetryClient, revision: u64) -> Result<TelemetrySnapshot, PollError> {
    let state = client.get("/state").map_err(PollError::Connection)?;
    let indicators = client.get("/indicators").map_err(PollError::Connection)?;
    let map_info = client
        .get("/map_info.json")
        .map_err(PollError::Connection)?;
    parse_and_normalize_with_map_info(revision, Instant::now(), &state, &indicators, &map_info)
        .map_err(PollError::Parse)
}

#[derive(Debug)]
enum PollError {
    Connection(ClientError),
    Parse(ParseError),
}

struct WorkerState {
    revision: u64,
    last_success: Option<Instant>,
    previous_status: Option<TelemetryStatus>,
    last_diagnostic: Option<(String, Instant)>,
    repeated_error_log_interval: Duration,
    backoff: Backoff,
}

impl WorkerState {
    fn new(poll_interval: Duration, repeated_error_log_interval: Duration) -> Self {
        Self {
            revision: 0,
            last_success: None,
            previous_status: None,
            last_diagnostic: None,
            repeated_error_log_interval,
            backoff: Backoff::new(poll_interval, poll_interval),
        }
    }

    fn next_revision(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }

    fn classify(
        &mut self,
        result: Result<TelemetrySnapshot, PollError>,
        now: Instant,
        config: &WorkerConfig,
    ) -> (TelemetrySnapshot, bool, Option<String>) {
        self.backoff.maximum = config.max_backoff;
        match result {
            Ok(snapshot)
                if matches!(
                    snapshot.status,
                    TelemetryStatus::Active | TelemetryStatus::Hangar
                ) =>
            {
                self.last_success = Some(now);
                (snapshot, true, None)
            }
            Ok(mut snapshot) => {
                if self.is_stale(now, config.stale_after) {
                    snapshot.status = TelemetryStatus::Stale;
                }
                (
                    snapshot,
                    false,
                    Some("battle telemetry is invalid".to_owned()),
                )
            }
            Err(error) => {
                let (base_status, diagnostic) = match error {
                    PollError::Connection(error) => (
                        TelemetryStatus::Disconnected,
                        format!("telemetry request failed: {error}"),
                    ),
                    PollError::Parse(error) => (
                        TelemetryStatus::Invalid,
                        format!("telemetry parse failed: {error}"),
                    ),
                };
                let status = if self.is_stale(now, config.stale_after) {
                    TelemetryStatus::Stale
                } else {
                    base_status
                };
                (
                    TelemetrySnapshot::with_status(
                        self.revision,
                        now,
                        status,
                        RawTelemetry::default(),
                    ),
                    false,
                    Some(diagnostic),
                )
            }
        }
    }

    fn is_stale(&self, now: Instant, threshold: Duration) -> bool {
        self.last_success
            .is_some_and(|last| now.saturating_duration_since(last) >= threshold)
    }

    fn log_transition(&mut self, status: TelemetryStatus, diagnostic: Option<&str>, now: Instant) {
        if self.previous_status != Some(status) {
            info!(?status, "telemetry status changed");
            self.previous_status = Some(status);
        }

        let Some(diagnostic) = diagnostic else {
            self.last_diagnostic = None;
            return;
        };
        let should_log = match &self.last_diagnostic {
            Some((previous, logged_at)) => {
                previous != diagnostic
                    || now.saturating_duration_since(*logged_at) >= self.repeated_error_log_interval
            }
            None => true,
        };
        if should_log {
            warn!("{diagnostic}");
            self.last_diagnostic = Some((diagnostic.to_owned(), now));
        }
    }
}

#[derive(Clone, Debug)]
struct Backoff {
    initial: Duration,
    maximum: Duration,
    failures: u32,
}

impl Backoff {
    fn new(initial: Duration, maximum: Duration) -> Self {
        Self {
            initial,
            maximum,
            failures: 0,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let shift = self.failures.min(31);
        self.failures = self.failures.saturating_add(1);
        self.initial
            .checked_mul(1_u32 << shift)
            .unwrap_or(self.maximum)
            .min(self.maximum)
    }

    fn reset(&mut self) {
        self.failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::mpsc;

    use super::*;

    struct FakeClient {
        responses: Mutex<VecDeque<Result<String, ClientError>>>,
    }

    impl TelemetryClient for FakeClient {
        fn get(&self, _path: &str) -> Result<String, ClientError> {
            self.responses.lock().unwrap().pop_front().unwrap()
        }
    }

    #[test]
    fn poll_reads_both_endpoints_and_normalizes() {
        let client = FakeClient {
            responses: Mutex::new(VecDeque::from([
                Ok(r#"{"valid":true,"IAS, km/h":101}"#.to_owned()),
                Ok(r#"{"valid":true,"AoA, deg":2.5}"#.to_owned()),
                Ok(r#"{"valid":true}"#.to_owned()),
            ])),
        };
        let snapshot = poll(&client, 9).unwrap();
        assert_eq!(snapshot.revision, 9);
        assert_eq!(snapshot.ias_kmh, Some(101.0));
        assert_eq!(snapshot.aoa_deg, Some(2.5));
    }

    #[test]
    fn exponential_backoff_is_capped_and_resets() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_millis(450));
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
        assert_eq!(backoff.next_delay(), Duration::from_millis(200));
        assert_eq!(backoff.next_delay(), Duration::from_millis(400));
        assert_eq!(backoff.next_delay(), Duration::from_millis(450));
        assert_eq!(backoff.next_delay(), Duration::from_millis(450));
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
    }

    #[test]
    fn failed_poll_becomes_stale_after_threshold() {
        let now = Instant::now();
        let config = WorkerConfig {
            stale_after: Duration::from_secs(1),
            ..WorkerConfig::default()
        };
        let mut state = WorkerState::new(config.poll_interval, config.repeated_error_log_interval);
        state.last_success = Some(now - Duration::from_secs(2));
        state.revision = 3;
        let (snapshot, succeeded, _) = state.classify(
            Err(PollError::Connection(ClientError::new("offline"))),
            now,
            &config,
        );
        assert!(!succeeded);
        assert_eq!(snapshot.status, TelemetryStatus::Stale);
        assert_eq!(snapshot.revision, 3);
    }

    #[test]
    fn success_resets_backoff() {
        let mut backoff = Backoff::new(Duration::from_millis(10), Duration::from_secs(1));
        let _ = backoff.next_delay();
        let _ = backoff.next_delay();
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_millis(10));
    }

    #[test]
    fn worker_publishes_and_shutdown_wakes_its_wait() {
        let client = FakeClient {
            responses: Mutex::new(VecDeque::from([
                Ok(r#"{"valid":true,"IAS, km/h":101}"#.to_owned()),
                Ok(r#"{"valid":true}"#.to_owned()),
                Ok(r#"{"valid":true}"#.to_owned()),
            ])),
        };
        let config = WorkerConfig {
            poll_interval: Duration::from_secs(30),
            max_backoff: Duration::from_secs(30),
            ..WorkerConfig::default()
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = TelemetryWorker::spawn_with_client(config, client, move |snapshot| {
            let _ = sender.try_send(snapshot);
        })
        .unwrap();

        let snapshot = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(snapshot.status, TelemetryStatus::Active);
        assert_eq!(snapshot.revision, 1);
        worker.shutdown().unwrap();
    }
}

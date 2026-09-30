use calloop::channel::{Channel, SyncSender, sync_channel};
use std::{
    fmt,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
        mpsc::{TryRecvError, TrySendError},
    },
};

/// A bounded, single-consumer latest-value handoff.
///
/// Publishing replaces the previous value and sends at most one pending wake.
/// The consumer never waits for the small producer-side mutex: `try_snapshot`
/// and `take_update` return `None` on contention.
pub struct Latest<T> {
    shared: Arc<Shared<T>>,
    wake_rx: Channel<()>,
}

struct Shared<T> {
    value: Mutex<LatestValue<T>>,
    next_revision: AtomicU64,
    wake_tx: SyncSender<()>,
}

#[derive(Debug)]
pub struct LatestValue<T> {
    pub revision: u64,
    pub value: Arc<T>,
}

impl<T> Clone for LatestValue<T> {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            value: Arc::clone(&self.value),
        }
    }
}

pub struct LatestPublisher<T> {
    shared: Arc<Shared<T>>,
}

#[derive(Clone)]
pub struct LatestReader<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for LatestPublisher<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionExhausted;

impl fmt::Display for RevisionExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("latest-value revision counter is exhausted")
    }
}

impl std::error::Error for RevisionExhausted {}

impl<T> Latest<T> {
    pub fn new(initial: T) -> Self {
        let (wake_tx, wake_rx) = sync_channel(1);
        Self {
            shared: Arc::new(Shared {
                value: Mutex::new(LatestValue {
                    revision: 0,
                    value: Arc::new(initial),
                }),
                next_revision: AtomicU64::new(0),
                wake_tx,
            }),
            wake_rx,
        }
    }

    pub fn publisher(&self) -> LatestPublisher<T> {
        LatestPublisher {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn reader(&self) -> LatestReader<T> {
        LatestReader {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Splits the consumer into a cloneable reader and the bounded calloop
    /// event source used to wake the UI loop.
    pub fn into_event_source(self) -> (LatestReader<T>, Channel<()>) {
        (
            LatestReader {
                shared: self.shared,
            },
            self.wake_rx,
        )
    }

    pub fn publish(&self, value: T) -> Result<u64, RevisionExhausted> {
        publish(&self.shared, value)
    }

    /// Returns the current value without consuming a wake notification.
    /// Returns `None` instead of waiting if a publisher is replacing it.
    pub fn try_snapshot(&self) -> Option<LatestValue<T>> {
        try_clone_value(&self.shared)
    }

    /// Consumes a single coalesced wake and returns the newest value.
    ///
    /// A notification is sent only after the updated value's lock is released,
    /// so observing a wake normally makes this lock-free from the UI's point of
    /// view. If another producer currently owns the lock, its subsequent wake
    /// keeps the update observable and this method returns `None` immediately.
    pub fn take_update(&self) -> Option<LatestValue<T>> {
        match self.wake_rx.try_recv() {
            Ok(()) => self.try_snapshot(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

impl<T> LatestPublisher<T> {
    pub fn publish(&self, value: T) -> Result<u64, RevisionExhausted> {
        publish(&self.shared, value)
    }

    pub fn try_snapshot(&self) -> Option<LatestValue<T>> {
        try_clone_value(&self.shared)
    }
}

impl<T> LatestReader<T> {
    pub fn try_snapshot(&self) -> Option<LatestValue<T>> {
        try_clone_value(&self.shared)
    }
}

fn publish<T>(shared: &Shared<T>, value: T) -> Result<u64, RevisionExhausted> {
    // Keep allocation and replacement in one critical section so concurrent
    // publishers cannot make the observable revision go backwards.
    let mut current = match shared.value.lock() {
        Ok(current) => current,
        Err(poisoned) => poisoned.into_inner(),
    };
    let revision = shared
        .next_revision
        .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| RevisionExhausted)?
        + 1;
    *current = LatestValue {
        revision,
        value: Arc::new(value),
    };
    drop(current);

    match shared.wake_tx.try_send(()) {
        Ok(()) | Err(TrySendError::Full(())) => Ok(revision),
        // The sender is owned by the same shared allocation as the receiver,
        // therefore disconnection is not expected while `Latest` is alive.
        Err(TrySendError::Disconnected(())) => Ok(revision),
    }
}

fn try_clone_value<T>(shared: &Shared<T>) -> Option<LatestValue<T>> {
    match shared.value.try_lock() {
        Ok(value) => Some(value.clone()),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner().clone()),
        Err(TryLockError::WouldBlock) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn starts_at_revision_zero_without_a_pending_wake() {
        let latest = Latest::new("initial");
        let snapshot = latest.try_snapshot().unwrap();
        assert_eq!(snapshot.revision, 0);
        assert_eq!(*snapshot.value, "initial");
        assert!(latest.take_update().is_none());
    }

    #[test]
    fn many_publications_coalesce_to_the_newest_value() {
        let latest = Latest::new(0);
        for value in 1..=1_000 {
            assert_eq!(latest.publish(value), Ok(value));
        }

        let update = latest.take_update().unwrap();
        assert_eq!(update.revision, 1_000);
        assert_eq!(*update.value, 1_000);
        assert!(latest.take_update().is_none());
    }

    #[test]
    fn calloop_event_source_is_bounded_and_reads_the_newest_value() {
        let latest = Latest::new(0);
        let publisher = latest.publisher();
        let (reader, wake) = latest.into_event_source();

        publisher.publish(1).unwrap();
        publisher.publish(2).unwrap();
        assert_eq!(wake.try_recv(), Ok(()));
        assert!(matches!(wake.try_recv(), Err(TryRecvError::Empty)));
        let snapshot = reader.try_snapshot().unwrap();
        assert_eq!(snapshot.revision, 2);
        assert_eq!(*snapshot.value, 2);
    }

    #[test]
    fn cloned_publishers_assign_unique_monotonic_revisions() {
        let latest = Latest::new(0_u64);
        let first = latest.publisher();
        let second = first.clone();
        let first_thread = thread::spawn(move || {
            (0..100)
                .map(|value| first.publish(value).unwrap())
                .collect::<Vec<_>>()
        });
        let second_thread = thread::spawn(move || {
            (100..200)
                .map(|value| second.publish(value).unwrap())
                .collect::<Vec<_>>()
        });

        let mut revisions = first_thread.join().unwrap();
        revisions.extend(second_thread.join().unwrap());
        revisions.sort_unstable();
        assert_eq!(revisions, (1..=200).collect::<Vec<_>>());
        assert_eq!(latest.try_snapshot().unwrap().revision, 200);
    }
}

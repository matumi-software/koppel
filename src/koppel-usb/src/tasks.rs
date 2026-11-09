use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{Notify, mpsc};

use crate::BoxFuture;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs fire-and-forget work so that a panic in it is logged, and so whoever stops can wait for
/// what is still running.
#[derive(Default)]
pub(crate) struct BackgroundTasks {
    running: Arc<Running>,
}

#[derive(Default)]
struct Running {
    count: AtomicUsize,
    idle: Notify,
}

impl BackgroundTasks {
    /// Starts `work`. A panic in it is logged with `name`.
    pub fn spawn(&self, name: impl Into<String>, work: impl Future<Output = ()> + Send + 'static) {
        let name = name.into();
        let running = self.running.clone();
        running.count.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            if let Err(e) = tokio::spawn(work).await
                && e.is_panic()
            {
                tracing::error!("Background task {name} failed: it panicked");
            }
            if running.count.fetch_sub(1, Ordering::SeqCst) == 1 {
                running.idle.notify_waiters();
            }
        });
    }

    /// Waits for what is running, up to `timeout`.
    pub async fn wait(&self, timeout: Duration) {
        let running = &self.running;
        let waited = tokio::time::timeout(timeout, async {
            loop {
                let idle = running.idle.notified();
                if running.count.load(Ordering::SeqCst) == 0 {
                    return;
                }
                idle.await;
            }
        })
        .await;
        if waited.is_err() {
            tracing::warn!(
                "{} background tasks were still running when they were asked to stop",
                running.count.load(Ordering::SeqCst)
            );
        }
    }
}

/// A wake-up that cannot pile up: any number of [`WakeSignal::set`] calls before a wait count as
/// one, so a burst of changes costs one pass of the loop that waits, not one per change.
pub struct WakeSignal {
    sender: mpsc::Sender<()>,
    receiver: tokio::sync::Mutex<mpsc::Receiver<()>>,
}

impl Default for WakeSignal {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self {
            sender,
            receiver: tokio::sync::Mutex::new(receiver),
        }
    }
}

impl WakeSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self) {
        let _ = self.sender.try_send(());
    }

    /// Forgets a wake-up nobody has waited for yet.
    pub fn reset(&self) {
        if let Ok(mut receiver) = self.receiver.try_lock() {
            let _ = receiver.try_recv();
        }
    }

    pub async fn wait(&self) {
        let _ = self.receiver.lock().await.recv().await;
    }

    /// Waits for [`WakeSignal::set`] or for `timeout`, whichever comes first.
    pub async fn wait_for(&self, timeout: Duration) {
        let _ = tokio::time::timeout(timeout, self.wait()).await;
    }
}

/// Lets a listener stop listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listener(u64);

type Handler<T> = Arc<dyn Fn(Arc<T>) -> BoxFuture<'static, ()> + Send + Sync>;

/// The listeners of one kind of change, called one after another.
pub(crate) struct Handlers<T> {
    what: &'static str,
    list: Mutex<Vec<(u64, Handler<T>)>>,
    next: AtomicU64,
}

impl<T: Send + Sync + 'static> Handlers<T> {
    pub fn new(what: &'static str) -> Self {
        Self {
            what,
            list: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
        }
    }

    pub fn add<F, Fut>(&self, handler: F) -> Listener
    where
        F: Fn(Arc<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let handler: Handler<T> = Arc::new(move |value| Box::pin(handler(value)));
        lock(&self.list).push((id, handler));
        Listener(id)
    }

    pub fn remove(&self, listener: Listener) {
        lock(&self.list).retain(|(id, _)| *id != listener.0);
    }

    /// Calls each listener in turn and waits for it. One that panics is logged and the rest are
    /// still called.
    pub async fn raise(&self, value: Arc<T>) {
        let handlers: Vec<Handler<T>> = lock(&self.list).iter().map(|(_, h)| h.clone()).collect();
        for handler in handlers {
            if let Err(e) = tokio::spawn(handler(value.clone())).await
                && e.is_panic()
            {
                tracing::warn!("A listener of {} failed: it panicked", self.what);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn many_sets_before_a_wait_wake_it_once() {
        let signal = Arc::new(WakeSignal::new());
        for _ in 0..50 {
            signal.set();
        }

        tokio::time::timeout(Duration::from_secs(5), signal.wait())
            .await
            .unwrap();

        // Nothing is left queued: the next wait is pending, where a counting semaphore would pass again.
        let waiting = signal.clone();
        let second = tokio::spawn(async move { waiting.wait().await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!second.is_finished());
        signal.set();
        tokio::time::timeout(Duration::from_secs(5), second)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn reset_forgets_a_wake_up_nobody_waited_for() {
        let signal = Arc::new(WakeSignal::new());
        signal.set();
        signal.reset();

        let waiting = signal.clone();
        let wait = tokio::spawn(async move { waiting.wait().await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(!wait.is_finished());
        signal.set();
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn a_wait_with_a_time_limit_returns_when_set() {
        let signal = Arc::new(WakeSignal::new());
        let waiting = signal.clone();
        let wait = tokio::spawn(async move { waiting.wait_for(Duration::from_secs(300)).await });

        signal.set();

        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn waiting_for_background_tasks_stops_at_the_timeout() {
        let tasks = BackgroundTasks::default();
        tasks.spawn("quick", async {});
        tasks.spawn("panics", async { panic!("on purpose") });
        tasks.wait(Duration::from_secs(5)).await;
        assert_eq!(tasks.running.count.load(Ordering::SeqCst), 0);

        tasks.spawn("slow", tokio::time::sleep(Duration::from_secs(60)));
        let started = std::time::Instant::now();
        tasks.wait(Duration::from_millis(200)).await;
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Everything one owner runs in the background: its tasks, and the token that tells them to end.
/// Stopping cancels the token, closes the tracker and waits for the tasks, up to a limit.
#[derive(Clone, Default)]
pub(crate) struct Tasks {
    tracker: TaskTracker,
    stopping: CancellationToken,
}

impl Tasks {
    /// Cancelled when the owner stops.
    pub fn token(&self) -> &CancellationToken {
        &self.stopping
    }

    /// Starts `work`. A panic in it is logged with `name`; the owner goes on.
    pub fn spawn(&self, name: impl Into<String>, work: impl Future<Output = ()> + Send + 'static) {
        let name = name.into();
        self.tracker.spawn(async move {
            let _log = PanicLog(name);
            work.await;
        });
    }

    /// Tells the tasks to end and waits for them, up to `timeout`.
    pub async fn stop(&self, timeout: Duration) {
        self.stopping.cancel();
        self.wait(timeout).await;
    }

    /// Waits for what is running, up to `timeout`, without cancelling anything.
    pub async fn wait(&self, timeout: Duration) {
        self.tracker.close();
        if tokio::time::timeout(timeout, self.tracker.wait())
            .await
            .is_err()
        {
            tracing::warn!(
                "{} background tasks were still running when they were asked to stop",
                self.tracker.len()
            );
        }
    }
}

/// Logs a panic of the task it lives in, as the unwinding drops it.
struct PanicLog(String);

impl Drop for PanicLog {
    fn drop(&mut self) {
        if std::thread::panicking() {
            tracing::error!("Background task {} failed: it panicked", self.0);
        }
    }
}

/// Lets a listener stop listening.
#[derive(Debug, Clone)]
pub struct Listener(CancellationToken);

impl Listener {
    pub(crate) fn stop(&self) {
        self.0.cancel();
    }
}

/// Calls `handler` with the newest value of `receiver` each time it changes, until the listener
/// or the owner stops. A value a newer one overtook while the handler was busy is skipped, so a
/// slow listener holds up only itself.
pub(crate) fn listen<T, F, Fut>(
    tasks: &Tasks,
    what: &'static str,
    mut receiver: watch::Receiver<T>,
    handler: F,
) -> Listener
where
    T: Clone + Send + Sync + 'static,
    F: Fn(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let stop = tasks.token().child_token();
    let listener = Listener(stop.clone());
    tasks.spawn(format!("a listener of {what}"), async move {
        loop {
            tokio::select! {
                changed = receiver.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _ = stop.cancelled() => return,
            }
            let value = receiver.borrow_and_update().clone();
            handler(value).await;
        }
    });
    listener
}

/// Waits 5, 10, 20 seconds and so on, up to a limit, and starts over after a success.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Backoff {
    min: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(min: Duration, max: Duration) -> Self {
        Self {
            min,
            max,
            next: min,
        }
    }

    /// How long to wait now; the wait after it is twice as long, up to the limit.
    pub fn next_wait(&mut self) -> Duration {
        let wait = self.next;
        self.next = (self.next * 2).min(self.max);
        wait
    }

    pub fn reset(&mut self) {
        self.next = self.min;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn waiting_for_tasks_stops_at_the_timeout() {
        let tasks = Tasks::default();
        tasks.spawn("quick", async {});
        tasks.spawn("slow", tokio::time::sleep(Duration::from_secs(60)));

        let started = tokio::time::Instant::now();
        tasks.wait(Duration::from_millis(200)).await;

        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_cancels_the_token_the_tasks_wait_on() {
        let tasks = Tasks::default();
        let token = tasks.token().clone();
        tasks.spawn("waits for the stop", async move { token.cancelled().await });

        let started = tokio::time::Instant::now();
        tasks.stop(Duration::from_secs(5)).await;

        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_task_does_not_stop_the_wait() {
        let tasks = Tasks::default();
        tasks.spawn("panics", async { panic!("on purpose") });

        tasks.wait(Duration::from_secs(5)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_listener_sees_the_newest_value_and_can_be_stopped() {
        let tasks = Tasks::default();
        let (sender, receiver) = watch::channel(0u32);
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let listener = listen(&tasks, "numbers", receiver, move |n| {
            let record = record.clone();
            async move { lock(&record).push(n) }
        });

        sender.send_replace(1);
        sender.send_replace(2);
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(*lock(&seen), vec![2]);

        listener.stop();
        tokio::time::sleep(Duration::from_millis(1)).await;
        sender.send_replace(3);
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(*lock(&seen), vec![2]);
        tasks.stop(Duration::from_secs(1)).await;
    }

    #[test]
    fn the_backoff_doubles_up_to_its_limit_and_starts_over() {
        let mut backoff = Backoff::new(Duration::from_secs(5), Duration::from_secs(60));

        let waits: Vec<u64> = (0..6).map(|_| backoff.next_wait().as_secs()).collect();
        backoff.reset();

        assert_eq!(waits, vec![5, 10, 20, 40, 60, 60]);
        assert_eq!(backoff.next_wait(), Duration::from_secs(5));
    }
}

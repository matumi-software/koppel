//! The tasks Koppel's own code starts: one [`CancellationToken`] and one [`TaskTracker`] for the
//! app, so a stop ends them all and waits for them, up to a limit, instead of each owner keeping
//! its own stop closure or leaving a task to the runtime.

use std::future::Future;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// How long a stop waits for the tasks that did not end when asked to.
pub const STOP_WAIT: Duration = Duration::from_secs(5);

/// A handle on the app's tasks. Cheap to clone; every clone is the same set.
#[derive(Clone, Default)]
pub struct Tasks {
    stop: CancellationToken,
    tracker: TaskTracker,
}

impl Tasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a task that ends when `work` does or when the app stops, whichever comes first.
    pub fn spawn<F>(&self, work: F) -> JoinHandle<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stop = self.stop.clone();
        self.tracker.spawn(async move {
            tokio::select! {
                () = stop.cancelled() => {}
                () = work => {}
            }
        })
    }

    /// Ends every task, then waits for all of them, at most `wait`.
    /// What is still running after that is aborted by the runtime when it drops. Returns how many
    /// tasks were left.
    pub async fn shutdown(&self, wait: Duration) -> usize {
        self.stop.cancel();
        self.tracker.close();
        if tokio::time::timeout(wait, self.tracker.wait())
            .await
            .is_err()
        {
            let left = self.tracker.len();
            tracing::warn!("{left} background task(s) did not end within {wait:?} of the stop");
            return left;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_stop_ends_what_listens_and_waits_for_it() {
        let tasks = Tasks::new();
        let (ended, finished) = tokio::sync::oneshot::channel::<()>();
        tasks.spawn(async move {
            std::future::pending::<()>().await;
            let _ = ended.send(());
        });
        let left = tasks.shutdown(STOP_WAIT).await;
        assert_eq!(left, 0);
        // The task's future was dropped, not run to the end.
        assert!(finished.await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_task_that_will_not_end_is_left_after_the_wait() {
        let tasks = Tasks::new();
        // One that ignores the stop, as only the tracker can start here.
        tasks.tracker.spawn(std::future::pending::<()>());
        let started = tokio::time::Instant::now();
        assert_eq!(tasks.shutdown(Duration::from_secs(2)).await, 1);
        assert_eq!(started.elapsed(), Duration::from_secs(2));
    }
}

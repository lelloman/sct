//! Own the fixed workers while retaining their existing abort/join order.
use simple_server::tasks::{ShutdownBehavior, TaskId, TaskSet};
use std::{convert::Infallible, future::Future, num::NonZeroUsize};

pub(super) struct BackgroundWorker {
    tasks: TaskSet<Infallible>,
    id: TaskId,
}

impl BackgroundWorker {
    pub(super) fn spawn(name: &str, work: impl Future<Output = ()> + Send + 'static) -> Self {
        let mut tasks = TaskSet::new(NonZeroUsize::new(1).unwrap());
        let id = tasks
            .spawn(name, ShutdownBehavior::FinishOnShutdown, |_| async move {
                work.await;
                Ok(())
            })
            .expect("one fixed worker in an active Tokio runtime");
        Self { tasks, id }
    }

    pub(super) async fn stop(&mut self) {
        self.tasks.close();
        let _ = self.tasks.abort_async(self.id);
        while self.tasks.join_next().await.is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Released(Arc<AtomicBool>);
    impl Drop for Released {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn stop_joins_cancelled_worker_before_releasing_resources() {
        let released = Arc::new(AtomicBool::new(false));
        let guard = Released(released.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        let mut worker = BackgroundWorker::spawn("held-resource", async move {
            let _guard = guard;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        assert!(!released.load(Ordering::SeqCst));
        worker.stop().await;
        assert!(released.load(Ordering::SeqCst));
        assert!(worker.tasks.is_empty());
        worker.stop().await;
    }

    #[tokio::test]
    async fn completed_or_panicked_workers_do_not_block_cleanup() {
        for panic in [false, true] {
            let (started, ready) = tokio::sync::oneshot::channel();
            let mut worker = BackgroundWorker::spawn("finished", async move {
                started.send(()).unwrap();
                assert!(!panic, "injected worker panic");
            });
            ready.await.unwrap();
            worker.stop().await;
            assert!(worker.tasks.is_empty());
        }
    }
}

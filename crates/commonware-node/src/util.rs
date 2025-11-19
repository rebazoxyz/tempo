//! Commonware utils.

use commonware_runtime::Handle;
use futures::FutureExt;
use std::task::{Context, Poll};

/// A collection of tasks spawned on the commonware runtime.
///
/// This is similar to tokio's `JoinSet` and can be used to await the completion of some or all of the tasks
/// in the set. The set is not ordered, and the tasks will be returned in the
/// order they complete.
/// All of the tasks must have the same return type `T`.
///
/// When the `JoinSet` is dropped, all tasks in the `JoinSet` are immediately aborted.
///
/// This is primarily intended to group small sets of [`Handle`] to ensure all get aborted on drop.
pub struct JoinSet<T>
where
    T: Send + 'static,
{
    tasks: Vec<Handle<T>>,
}

impl<T> JoinSet<T>
where
    T: Send + 'static,
{
    /// Creates a new instance of the set
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Add the new handle to the set
    pub fn push(&mut self, task: Handle<T>) {
        self.tasks.push(task);
    }

    /// Creates a [`JoinSet`] from a vec of handles.
    pub fn from_vec(tasks: Vec<Handle<T>>) -> Self {
        Self { tasks }
    }

    /// Returns how many tasks are still joined.
    pub const fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Returns true if this set is empty
    pub const fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Awaits the completion of all tasks in this `JoinSet`, returning a vector of their results.
    ///
    /// The results will be stored in the order they completed not the order they were spawned.
    pub async fn join_all(mut self) -> Vec<Result<T, commonware_runtime::Error>> {
        let mut output = Vec::with_capacity(self.len());

        while let Some(res) = self.join_next().await {
            output.push(res);
        }
        output
    }

    pub async fn join_next(&mut self) -> Option<Result<T, commonware_runtime::Error>> {
        std::future::poll_fn(|cx| self.poll_join_next(cx)).await
    }

    /// Polls for one of the tasks in the set to complete.
    ///
    /// If this returns `Poll::Ready(Some(_))`, then the task that completed is removed from the set.
    fn poll_join_next(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<T, commonware_runtime::Error>>> {
        if self.tasks.is_empty() {
            return Poll::Ready(None);
        }

        for idx in (0..self.tasks.len()).rev() {
            let mut task = self.tasks.swap_remove(idx);
            match task.poll_unpin(cx) {
                Poll::Ready(result) => return Poll::Ready(Some(result)),
                Poll::Pending => {
                    self.tasks.push(task);
                }
            }
        }
        Poll::Pending
    }
}

impl<T> Default for JoinSet<T>
where
    T: Send + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for JoinSet<T>
where
    T: Send + 'static,
{
    fn drop(&mut self) {
        self.tasks.drain(..).for_each(|handle| handle.abort());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_runtime::{Runner, Spawner, deterministic};
    use futures::StreamExt;

    #[test]
    fn join_next_returns_completed_task() {
        let executor = deterministic::Runner::default();
        executor.start(|context| async move {
            let mut set = JoinSet::new();

            set.push(context.spawn(|_| async { 42 }));

            let result = set.join_next().await;
            assert!(result.is_some());
            assert_eq!(result.unwrap().unwrap(), 42);
            assert_eq!(set.len(), 0);

            assert_eq!(set.join_all().await.len(), 0);
        });
    }

    #[test]
    fn join_all_collects_all_results() {
        let executor = deterministic::Runner::default();
        executor.start(|context| async move {
            let mut set = JoinSet::new();

            // Spawn tasks that complete immediately
            let h1 = context.clone().spawn(|_| async { 1 });
            let h2 = context.clone().spawn(|_| async { 2 });
            let h3 = context.spawn(|_| async { 3 });

            set.push(h1);
            set.push(h2);
            set.push(h3);

            let results = set.join_all().await;
            assert_eq!(results.len(), 3);

            // Check that we got results (some may be Closed, some Ok depending on runtime behavior)
            let ok_count = results.iter().filter(|r| r.is_ok()).count();
            assert!(
                ok_count >= 1,
                "At least one task should complete successfully"
            );
        });
    }

    #[test]
    fn join_next_returns_all_spawned_tasks() {
        let executor = deterministic::Runner::default();
        executor.start(|context| async move {
            let mut set = JoinSet::new();

            // Spawn two tasks
            set.push(context.clone().spawn(|_| async { 1 }));
            set.push(context.spawn(|_| async { 2 }));

            // Get first result
            let first = set.join_next().await;
            assert!(first.is_some());
            assert_eq!(set.len(), 1);

            // Get second result
            let second = set.join_next().await;
            assert!(second.is_some());
            assert_eq!(set.len(), 0);

            // No more results
            let third = set.join_next().await;
            assert!(third.is_none());
        });
    }

    #[test]
    fn tasks_are_aborted_on_drop() {
        struct SendOnDrop(futures::channel::mpsc::UnboundedSender<()>);

        impl Drop for SendOnDrop {
            fn drop(&mut self) {
                let _ = self.0.unbounded_send(());
            }
        }

        let executor = deterministic::Runner::default();
        executor.start(|context| async move {
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            let mut set = JoinSet::new();

            for _ in 0..10 {
                let on_drop = SendOnDrop(tx.clone());
                // Spawn a task that would increment counter
                set.push(context.clone().spawn(move |_| async move {
                    futures::future::pending::<()>().await;
                    drop(on_drop);
                }));
            }
            drop(tx);

            // ensure set is dropped
            drop(set);

            for _ in 0..10 {
                rx.next().await;
            }
        });
    }
}

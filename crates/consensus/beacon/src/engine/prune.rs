//! Prune management for the engine implementation.

use futures::{FutureExt, Stream};
use reth_provider::CanonStateNotification;
use reth_prune::{Pruner, PrunerError, PrunerWithResult};
use reth_tasks::TaskSpawner;
use std::task::{ready, Context, Poll};
use tokio::sync::oneshot;

/// Manages pruning under the control of the engine.
///
/// This type controls the [Pruner].
pub(crate) struct EnginePruneController<St> {
    /// The current state of the pruner.
    pruner_state: PrunerState<St>,
    /// The type that can spawn the pruner task.
    pruner_task_spawner: Box<dyn TaskSpawner>,
}

impl<St> EnginePruneController<St>
where
    St: Stream<Item = CanonStateNotification> + Send + Unpin + 'static,
{
    /// Create a new instance
    pub(crate) fn new(pruner: Pruner<St>, pruner_task_spawner: Box<dyn TaskSpawner>) -> Self {
        Self { pruner_state: PrunerState::Idle(Some(pruner)), pruner_task_spawner }
    }

    /// Returns `true` if the pruner is idle.
    pub(crate) fn is_pruner_idle(&self) -> bool {
        self.pruner_state.is_idle()
    }

    /// Returns `true` if the pruner is active.
    pub(crate) fn is_pruner_active(&self) -> bool {
        !self.is_pruner_idle()
    }

    /// Advances the pruner state.
    ///
    /// This checks for the result in the channel, or returns pending if the pruner is idle.
    fn poll_pruner(&mut self, cx: &mut Context<'_>) -> Poll<EnginePruneEvent> {
        let res = match self.pruner_state {
            PrunerState::Idle(_) => return Poll::Pending,
            PrunerState::Running(ref mut fut) => {
                ready!(fut.poll_unpin(cx))
            }
        };
        let ev = match res {
            Ok((pruner, result)) => {
                self.pruner_state = PrunerState::Idle(Some(pruner));
                EnginePruneEvent::Finished { result }
            }
            Err(_) => {
                // failed to receive the pruner
                EnginePruneEvent::TaskDropped
            }
        };
        Poll::Ready(ev)
    }

    /// This will spawn the pruner if it is idle.
    fn try_spawn_pruner(&mut self) -> Option<EnginePruneEvent> {
        match &mut self.pruner_state {
            PrunerState::Idle(pruner) => {
                let pruner = pruner.take()?;

                let (tx, rx) = oneshot::channel();
                self.pruner_task_spawner.spawn_critical_blocking(
                    "pruner task",
                    Box::pin(async move {
                        let result = pruner.run_as_fut().await;
                        let _ = tx.send(result);
                    }),
                );
                self.pruner_state = PrunerState::Running(rx);

                Some(EnginePruneEvent::Started)
            }
            PrunerState::Running(_) => None,
        }
    }

    /// Advances the prune process.
    pub(crate) fn poll(&mut self, cx: &mut Context<'_>) -> Poll<EnginePruneEvent> {
        // Try to spawn a pruner
        if let Some(event) = self.try_spawn_pruner() {
            return Poll::Ready(event)
        }

        loop {
            if let Poll::Ready(event) = self.poll_pruner(cx) {
                return Poll::Ready(event)
            }

            if !self.pruner_state.is_idle() {
                // Can not make any progress
                return Poll::Pending
            }
        }
    }
}

/// The event type emitted by the [EnginePruneController].
#[derive(Debug)]
pub(crate) enum EnginePruneEvent {
    /// Pruner started
    Started,
    /// Pruner finished
    ///
    /// If this is returned, the pruner is idle.
    Finished {
        /// Final result of the pruner run.
        result: Result<(), PrunerError>,
    },
    /// Pruner task was dropped after it was started, unable to receive it because channel
    /// closed. This would indicate a panicked pruner task
    TaskDropped,
}

/// The possible pruner states within the sync controller.
///
/// [PrunerState::Idle] means that the pruner is currently idle.
/// [PrunerState::Running] means that the pruner is currently running.
///
/// NOTE: The differentiation between these two states is important, because when the pruner is
/// running, it acquires the write lock over the database. This means that we cannot forward to the
/// blockchain tree any messages that would result in database writes, since it would result in a
/// deadlock.
enum PrunerState<St> {
    /// Pruner is idle.
    Idle(Option<Pruner<St>>),
    /// Pruner is running and waiting for a response
    Running(oneshot::Receiver<PrunerWithResult<St>>),
}

impl<St> PrunerState<St> {
    /// Returns `true` if the state matches idle.
    fn is_idle(&self) -> bool {
        matches!(self, PrunerState::Idle(_))
    }
}
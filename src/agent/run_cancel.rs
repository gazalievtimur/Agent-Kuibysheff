use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

/// Cooperative cancellation and wall-clock deadline for one agent run.
///
/// Shared from the composition root into the agent loop, MCP actors, and
/// `home.run` (sandbox deadline clamp). Cancelling the token stops new work;
/// in-flight tool side effects may still complete (cancel-safety tradeoff).
#[derive(Clone, Debug)]
pub struct RunCancel {
    token: CancellationToken,
    deadline: Arc<RwLock<Option<Instant>>>,
}

impl Default for RunCancel {
    fn default() -> Self {
        Self::new()
    }
}

impl RunCancel {
    /// Creates an unarmed cancel handle (token never cancels until [`Self::cancel`]
    /// or [`Self::arm_deadline`]).
    #[must_use]
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            deadline: Arc::new(RwLock::new(None)),
        }
    }

    /// Shared cancellation token for `select!` / cooperative checks.
    #[must_use]
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Arms a wall-clock deadline from now and cancels the token when it elapses.
    ///
    /// Call once at the start of [`crate::agent::AgentEngine::run`]. Replacing an
    /// already-armed deadline updates the stored instant but leaves any prior
    /// sleeper task running (it becomes a no-op if the token is already cancelled).
    pub fn arm_deadline(&self, max_duration: Duration) {
        let deadline = Instant::now() + max_duration;
        *self
            .deadline
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(deadline);
        let token = self.token.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(max_duration) => {
                    token.cancel();
                }
                () = token.cancelled() => {}
            }
        });
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Remaining time until the armed deadline.
    ///
    /// Returns `None` when no deadline was armed. When the deadline has already
    /// passed, returns a 1 ms remainder so sandbox clamps still force a quick kill.
    #[must_use]
    pub fn remaining(&self) -> Option<Duration> {
        let guard = self
            .deadline
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = (*guard)?;
        Some(
            deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::from_millis(1)),
        )
    }

    /// Requests cancellation (tests and future external signals).
    pub fn cancel(&self) {
        self.token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn arm_deadline_cancels_token() {
        let cancel = RunCancel::new();
        cancel.arm_deadline(Duration::from_millis(20));
        tokio::time::timeout(Duration::from_secs(2), cancel.token().cancelled())
            .await
            .expect("deadline should cancel token");
        assert!(cancel.is_cancelled());
        assert_eq!(cancel.remaining(), Some(Duration::from_millis(1)));
    }
}

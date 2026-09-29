//! The one deadline of the current tool call. The dispatcher sets it when a
//! call is accepted; adapters read it for limiter waits, retries and
//! `Retry-After` without it passing through the frozen provider traits.

use std::future::Future;

use tokio::time::Instant;

use super::defaults::CALL_DEADLINE;

tokio::task_local! {
    static DEADLINE: Instant;
}

/// Run `future` with `deadline` as the call deadline.
pub async fn with_deadline<F: Future>(deadline: Instant, future: F) -> F::Output {
    DEADLINE.scope(deadline, future).await
}

/// The current call's deadline; outside a call, [`CALL_DEADLINE`] from now.
/// Read it once per operation, not in a loop.
pub fn deadline() -> Instant {
    DEADLINE
        .try_with(|deadline| *deadline)
        .unwrap_or_else(|_| Instant::now() + CALL_DEADLINE)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn scoped_deadline_is_visible_inside_only() {
        let set = Instant::now() + Duration::from_secs(3);
        assert_eq!(with_deadline(set, async { deadline() }).await, set);
        assert_eq!(deadline(), Instant::now() + CALL_DEADLINE);
    }
}

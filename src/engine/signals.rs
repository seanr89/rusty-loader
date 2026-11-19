//! Signal handling and graceful cancellation listener (`Ctrl+C` / `SIGINT`).

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Waits for a `SIGINT` (`Ctrl+C`) signal from the OS, prints a graceful shutdown
/// message to `stderr`, and triggers `token.cancel()`.
pub async fn wait_for_shutdown_signal(token: CancellationToken) {
    if let Ok(()) = tokio::signal::ctrl_c().await {
        eprintln!("\nReceived Ctrl+C, initiating graceful shutdown...");
        token.cancel();
    }
}

/// Spawns a background task that listens for `Ctrl+C` and cancels `token`,
/// exiting automatically if `token` is cancelled externally before `Ctrl+C` arrives.
pub fn spawn_shutdown_listener(token: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = token.cancelled() => {}
            _ = wait_for_shutdown_signal(token.clone()) => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_spawn_shutdown_listener_exits_when_token_cancelled() {
        let token = CancellationToken::new();
        let handle = spawn_shutdown_listener(token.clone());

        token.cancel();

        let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
        assert!(result.is_ok(), "shutdown listener should exit cleanly when token is cancelled");
    }
}

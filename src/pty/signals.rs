use portable_pty::{MasterPty, PtySize};
use signal_hook::consts::SIGWINCH;
use signal_hook_tokio::Signals;
use tokio_stream::StreamExt;

/// Forward SIGWINCH signals to the PTY master so the child shell sees terminal resizes.
pub async fn forward_sigwinch(master: Box<dyn MasterPty + Send>) {
    let mut signals = match Signals::new([SIGWINCH]) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("failed to register SIGWINCH handler: {e}");
            return;
        }
    };

    while let Some(signal) = signals.next().await {
        if signal == SIGWINCH {
            match crossterm::terminal::size() {
                Ok((cols, rows)) => {
                    let _ = master.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                    tracing::debug!(rows, cols, "forwarded SIGWINCH");
                }
                Err(e) => {
                    tracing::warn!("failed to get terminal size on SIGWINCH: {e}");
                }
            }
        }
    }
}

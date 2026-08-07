//! Progress reporting for long-running operations.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// A progress update from an in-flight operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Progress {
    /// Work began on a named item.
    Started {
        item: String,
    },
    /// A raw output line from a child process.
    ///
    /// Streamed to the log pane for diagnostics and *never* parsed for control flow —
    /// package-manager output is localized and reflows unpredictably. Decisions are
    /// made from exit codes only.
    Log {
        line: String,
    },
    /// Fractional completion, when the underlying tool reports something usable.
    Percent {
        item: String,
        percent: u8,
    },
    Finished {
        item: String,
        outcome: Outcome,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Success,
    /// Nothing to do — already in the desired state.
    Skipped {
        reason: String,
    },
    Failed {
        message: String,
    },
}

impl Outcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success | Self::Skipped { .. })
    }
}

/// Where an operation sends its progress.
///
/// A cloneable, non-blocking sink. Sends are deliberately fire-and-forget: a stalled or
/// dropped UI must never block or fail a system operation that is already underway.
#[derive(Debug, Clone)]
pub struct ProgressSink(Option<mpsc::UnboundedSender<Progress>>);

impl ProgressSink {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Progress>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self(Some(tx)), rx)
    }

    /// A sink that discards everything. For tests and for callers that don't care.
    pub fn null() -> Self {
        Self(None)
    }

    pub fn send(&self, progress: Progress) {
        if let Some(tx) = &self.0 {
            // Ignore send errors: a closed receiver means the UI went away, which is
            // not a reason to abort a half-finished registry write.
            let _ = tx.send(progress);
        }
    }

    pub fn log(&self, line: impl Into<String>) {
        self.send(Progress::Log { line: line.into() });
    }

    pub fn started(&self, item: impl Into<String>) {
        self.send(Progress::Started { item: item.into() });
    }

    pub fn finished(&self, item: impl Into<String>, outcome: Outcome) {
        self.send(Progress::Finished {
            item: item.into(),
            outcome,
        });
    }
}

impl Default for ProgressSink {
    fn default() -> Self {
        Self::null()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_swallows_everything_without_panicking() {
        let sink = ProgressSink::null();
        sink.log("anything");
        sink.started("x");
        sink.finished("x", Outcome::Success);
    }

    #[tokio::test]
    async fn messages_arrive_in_order() {
        let (sink, mut rx) = ProgressSink::new();
        sink.started("firefox");
        sink.log("downloading");
        sink.finished("firefox", Outcome::Success);

        assert_eq!(
            rx.recv().await.unwrap(),
            Progress::Started {
                item: "firefox".into()
            }
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            Progress::Log {
                line: "downloading".into()
            }
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            Progress::Finished {
                item: "firefox".into(),
                outcome: Outcome::Success
            }
        );
    }

    #[tokio::test]
    async fn sending_after_the_receiver_drops_does_not_panic() {
        // The UI closing mid-install must not take down an in-flight system operation.
        let (sink, rx) = ProgressSink::new();
        drop(rx);
        sink.log("still running");
        sink.finished("x", Outcome::Success);
    }

    #[test]
    fn skipped_counts_as_success() {
        assert!(Outcome::Skipped {
            reason: "already installed".into()
        }
        .is_success());
        assert!(!Outcome::Failed {
            message: "boom".into()
        }
        .is_success());
    }
}

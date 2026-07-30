use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const FRAME_INTERVAL: Duration = Duration::from_millis(80);
const HIDE_CURSOR: &str = "\u{1b}[?25l";
const CLEAR_AND_SHOW_CURSOR: &str = "\r\u{1b}[K\u{1b}[?25h";

#[derive(Debug)]
pub struct Spinner {
    active: Option<ActiveSpinner>,
}

#[derive(Debug)]
struct ActiveSpinner {
    cancellation: CancellationToken,
    task: JoinHandle<()>,
}

impl Spinner {
    #[must_use]
    pub fn start(label: &str) -> Self {
        if !io::stderr().is_terminal() {
            write_stderr(&fallback_text(label));
            return Self { active: None };
        }

        write_stderr(HIDE_CURSOR);
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let label = label.to_owned();
        let task = tokio::spawn(async move {
            let mut frame_index = 0;
            loop {
                tokio::select! {
                    () = task_cancellation.cancelled() => break,
                    () = tokio::time::sleep(FRAME_INTERVAL) => {
                        write_stderr(&frame_text(FRAMES[frame_index % FRAMES.len()], &label));
                        frame_index += 1;
                    }
                }
            }
        });
        Self {
            active: Some(ActiveSpinner { cancellation, task }),
        }
    }

    pub async fn stop(mut self) {
        if let Some(active) = self.active.take() {
            active.cancellation.cancel();
            let _ = active.task.await;
            write_stderr(CLEAR_AND_SHOW_CURSOR);
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancellation.cancel();
            write_stderr(CLEAR_AND_SHOW_CURSOR);
        }
    }
}

fn frame_text(frame: &str, label: &str) -> String {
    format!("\r{frame} {label}...")
}

fn fallback_text(label: &str) -> String {
    format!("{label}...\n")
}

fn write_stderr(value: &str) {
    let mut output = io::stderr().lock();
    let _ = output
        .write_all(value.as_bytes())
        .and_then(|()| output.flush());
}

#[cfg(test)]
mod tests {
    use super::{fallback_text, frame_text};

    #[test]
    fn preserves_the_original_spinner_display() {
        assert_eq!(frame_text("⠋", ""), "\r⠋ ...");
        assert_eq!(frame_text("⠙", "Working"), "\r⠙ Working...");
        assert_eq!(fallback_text(""), "...\n");
        assert_eq!(fallback_text("Working"), "Working...\n");
    }
}

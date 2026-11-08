//! Non-Unix builds: no single-instance socket and no `--toggle`.

use crate::events::EventSender;

pub enum Outcome {
    NotRunning,
    #[allow(dead_code)]
    Handled,
    #[allow(dead_code)]
    Replaced,
}

pub fn build_id() -> String {
    String::new()
}

pub fn send_to_running(_command: &str, _build: &str) -> Outcome {
    Outcome::NotRunning
}

pub fn listen(_events: EventSender, _build: String) {}

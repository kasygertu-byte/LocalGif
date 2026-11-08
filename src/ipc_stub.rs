//! Non-Unix builds: no single-instance socket and no `--toggle`.

use crate::events::EventSender;

pub fn send_to_running(_command: &str) -> bool {
    false
}

pub fn listen(_events: EventSender) {}

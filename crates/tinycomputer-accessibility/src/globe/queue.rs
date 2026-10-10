//! Native bounded event retention with explicit continuity loss.
use std::collections::VecDeque;

pub(super) const CAPACITY: usize = 64;
#[derive(Debug, Default)]
pub(super) struct Queue {
    events: VecDeque<String>,
    overflow: bool,
}
impl Queue {
    pub(super) fn discontinuity(&mut self) {
        self.overflow = true;
    }
    pub(super) fn push(&mut self, event: String) {
        if !matches!(event.as_str(), "FN_DOWN" | "FN_UP") {
            self.overflow = true;
            return;
        }
        if self.events.len() == CAPACITY {
            self.events.pop_front();
            self.overflow = true;
        }
        self.events.push_back(event);
    }
    pub(super) fn len(&self) -> usize {
        self.events.len()
    }
    pub(super) fn drain(&mut self) -> (Vec<String>, bool) {
        let overflow = std::mem::take(&mut self.overflow);
        (self.events.drain(..).collect(), overflow)
    }
}
#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;

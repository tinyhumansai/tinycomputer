//! Bounded physical-event framing; helper output never becomes a diagnostic payload.
use std::io::BufRead;
const MAX_LINE: usize = 128;

pub(super) fn events<R: BufRead>(
    mut reader: R,
    mut event: impl FnMut(&str),
) -> Result<(), &'static str> {
    let mut line = Vec::new();
    loop {
        let bytes = reader.fill_buf().map_err(|_| "native_read_failed")?;
        if bytes.is_empty() {
            return if line.is_empty() {
                Err("native_stream_ended")
            } else {
                Err("native_truncated_event")
            };
        }
        let count = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |n| n + 1);
        if count > MAX_LINE.saturating_sub(line.len()) {
            return Err("native_frame_limit");
        }
        line.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if line.last() == Some(&b'\n') {
            let value = std::str::from_utf8(&line)
                .map_err(|_| "native_invalid_event")?
                .trim();
            if !value.is_empty() {
                event(value);
            }
            line.clear();
        }
    }
}

pub(super) fn events_into_queue<R: BufRead>(
    reader: R,
    queue: &std::sync::Mutex<super::queue::Queue>,
) -> Result<(), &'static str> {
    let mut queue_failed = false;
    let result = events(reader, |event| match queue.lock() {
        Ok(mut guard) => guard.push(event.to_owned()),
        Err(_) => queue_failed = true,
    });
    let mut guard = queue.lock().map_err(|_| "native_queue_failed")?;
    if result.is_err() || queue_failed {
        guard.discontinuity();
    }
    if queue_failed {
        Err("native_queue_failed")
    } else {
        result
    }
}

pub(super) fn errors(
    mut reader: impl std::io::Read,
    mut report: impl FnMut(),
) -> Result<(), &'static str> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer).map_err(|_| "native_read_failed")?;
        if count == 0 {
            return Ok(());
        }
        report();
    }
}
#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;

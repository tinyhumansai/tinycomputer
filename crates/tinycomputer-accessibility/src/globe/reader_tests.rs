//! Exact native facts and bounded malformed streams.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "controlled native fixtures assert completion and worker panic handling"
)]
use super::*;
#[test]
fn physical_lines_are_bounded_and_truncated_payloads_are_not_published() {
    let mut seen = Vec::new();
    events(b"FN_DOWN\n\nFN_UP\n".as_slice(), |event| {
        seen.push(event.to_owned());
    })
    .expect("facts");
    assert_eq!(seen, ["FN_DOWN", "FN_UP"]);
    assert_eq!(
        events(vec![b'x'; MAX_LINE + 1].as_slice(), |_| panic!(
            "oversize published"
        )),
        Err("native_frame_limit")
    );
    assert_eq!(
        events(b"FN_DOWN".as_slice(), |_| panic!("truncated published")),
        Err("native_truncated_event")
    );
    assert_eq!(
        events(b"\xff\n".as_slice(), |_| panic!("invalid UTF8 published")),
        Err("native_invalid_event")
    );
}
#[test]
fn stderr_is_discarded_in_fixed_chunks_without_building_lines() {
    let mut reports = 0;
    errors(vec![b'x'; 40_000].as_slice(), || reports += 1).expect("bounded drain");
    assert_eq!(reports, 5);
}

#[test]
fn native_pipe_read_failures_are_fixed_errors_without_payload_publication() {
    struct Unreadable;
    impl std::io::Read for Unreadable {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }
    impl std::io::BufRead for Unreadable {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn consume(&mut self, _: usize) {}
    }
    assert_eq!(
        events(Unreadable, |_| panic!("failed stream published")),
        Err("native_read_failed")
    );
    assert_eq!(
        errors(Unreadable, || panic!("failed stream reported content")),
        Err("native_read_failed")
    );
}

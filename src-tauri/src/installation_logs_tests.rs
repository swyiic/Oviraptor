use super::*;
use std::io::Cursor;

fn events(bytes: &[u8]) -> Vec<Event> {
    let (sender, receiver) = channel();
    forward(Cursor::new(bytes), "stdout", sender);
    receiver.into_iter().collect()
}

fn line(message: &str) -> Event {
    Event::Line {
        stream: "stdout",
        message: message.into(),
    }
}

#[test]
fn installation_logs_preserve_unicode_repetition_cr_progress_and_final_record() {
    assert_eq!(
        events("准备\r\n10%\r20%\n\n完成\n完成\n末尾".as_bytes()),
        [
            line("准备"),
            line("10%"),
            line("20%"),
            line("完成"),
            line("完成"),
            line("末尾")
        ]
    );
    assert!(events(b"").is_empty());
    assert!(events(b" \r\n\t\n").is_empty());
}

#[test]
fn installation_logs_redact_before_queueing_and_bound_display_length() {
    let mut bytes = b"\x1b[32mFOFA_KEY=private-fofa password=private-password\x1b[0m\n".to_vec();
    bytes.extend_from_slice("界".repeat(1300).as_bytes());
    let output = events(&bytes);
    assert_eq!(output.len(), 2);
    let Event::Line { message, .. } = &output[0] else {
        panic!("expected line")
    };
    assert!(!message.contains("private-"));
    assert!(!message.contains('\x1b'));
    let Event::Line { message, .. } = &output[1] else {
        panic!("expected line")
    };
    assert_eq!(message.chars().count(), 1200);
}

#[test]
fn installation_logs_discard_entire_oversized_records_and_resume_at_delimiter() {
    let mut bytes = b"private-prefix".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', MAX_RECORD_BYTES * 4));
    bytes.extend_from_slice(b"private-suffix\r\nafter\n");
    assert_eq!(events(&bytes), [line(OMITTED_RECORD), line("after")]);
    bytes.truncate(bytes.len() - b"\r\nafter\n".len());
    assert_eq!(events(&bytes), [line(OMITTED_RECORD)]);
    assert_eq!(
        events(&vec![b'a'; MAX_RECORD_BYTES]),
        [line(&"a".repeat(1200))]
    );
    assert_eq!(
        events(&vec![b'a'; MAX_RECORD_BYTES + 1]),
        [line(OMITTED_RECORD)]
    );
}

struct Fragmented {
    bytes: Cursor<Vec<u8>>,
    interrupt: bool,
}
impl Read for Fragmented {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.interrupt {
            self.interrupt = false;
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.interrupt = true;
        self.bytes.read(&mut output[..1])
    }
}

#[test]
fn installation_logs_retry_interrupted_reads_and_reassemble_utf8_fragments() {
    let (sender, receiver) = channel();
    forward(
        Fragmented {
            bytes: Cursor::new("中文🙂\r\nnext".as_bytes().to_vec()),
            interrupt: true,
        },
        "stdout",
        sender,
    );
    assert_eq!(
        receiver.into_iter().collect::<Vec<_>>(),
        [line("中文🙂"), line("next")]
    );
    assert_eq!(events(b"bad\xffutf8\n"), [line("bad\u{fffd}utf8")]);
}

struct Broken {
    first: bool,
}
impl Read for Broken {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.first {
            self.first = false;
            let bytes = b"complete\nprivate-incomplete";
            output[..bytes.len()].copy_from_slice(bytes);
            return Ok(bytes.len());
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "private-error-detail",
        ))
    }
}

#[test]
fn installation_logs_report_read_errors_without_partial_text_or_raw_error() {
    let (sender, receiver) = channel();
    forward(Broken { first: true }, "stdout", sender);
    assert_eq!(
        receiver.into_iter().collect::<Vec<_>>(),
        [
            line("complete"),
            Event::ReadFailed {
                stream: "stdout",
                kind: io::ErrorKind::UnexpectedEof
            }
        ]
    );
}

#[test]
fn installation_logs_channel_capacity_is_bounded_and_disconnect_stops_reader() {
    let (sender, receiver) = channel();
    for _ in 0..QUEUE_CAPACITY {
        sender.try_send(line("ok")).unwrap();
    }
    assert!(matches!(
        sender.try_send(line("overflow")),
        Err(mpsc::TrySendError::Full(_))
    ));
    drop(receiver);
    struct StopAfterLine;
    impl Read for StopAfterLine {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            output[..3].copy_from_slice(b"ok\n");
            Ok(3)
        }
    }
    // A disconnected receiver must return instead of draining an endless pipe.
    forward(StopAfterLine, "stdout", sender);
}

#[test]
fn installation_logs_large_unterminated_stream_uses_bounded_reads_and_one_marker() {
    struct Huge {
        remaining: usize,
    }
    impl Read for Huge {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            assert!(output.len() <= 8192);
            let length = output.len().min(self.remaining);
            output[..length].fill(b'x');
            self.remaining -= length;
            Ok(length)
        }
    }
    let (sender, receiver) = channel();
    forward(
        Huge {
            remaining: 4 * 1024 * 1024,
        },
        "stdout",
        sender,
    );
    assert_eq!(
        receiver.into_iter().collect::<Vec<_>>(),
        [line(OMITTED_RECORD)]
    );
}

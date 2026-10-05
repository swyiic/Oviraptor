//! Bounded, display-only framing for administrator installation output.
//! Never split an oversized record into fragments: that could disclose a
//! credential whose redaction prefix was in a preceding fragment.
use std::io::{self, BufRead, BufReader, Read};
use std::sync::mpsc::{self, Receiver, SyncSender};

mod journal;
mod process;
pub(crate) use journal::{page, Journal, Page};
pub(crate) use process::run_command;

const MAX_RECORD_BYTES: usize = 16 * 1024;
const QUEUE_CAPACITY: usize = 64;
const OMITTED_RECORD: &str = "[安装输出单条记录超过 16384 字节，已省略整条内容]";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Line {
        stream: &'static str,
        message: String,
    },
    ReadFailed {
        stream: &'static str,
        kind: io::ErrorKind,
    },
}

pub(crate) fn channel() -> (SyncSender<Event>, Receiver<Event>) {
    mpsc::sync_channel(QUEUE_CAPACITY)
}

fn emit(sender: &SyncSender<Event>, stream: &'static str, bytes: &[u8]) -> bool {
    let message = crate::log_display::line(&String::from_utf8_lossy(bytes));
    message.is_empty() || sender.send(Event::Line { stream, message }).is_ok()
}

/// Backpressure bounds queued display messages. Input buffering is independently
/// bounded, including streams that never print a newline. CR is a progress-record
/// delimiter, and the LF in CRLF cannot create a duplicate empty event.
pub(crate) fn forward(reader: impl Read, stream: &'static str, sender: SyncSender<Event>) {
    let mut reader = BufReader::with_capacity(8192, reader);
    let mut framer = crate::log_framing::Framer::new(MAX_RECORD_BYTES);
    let mut emit_record = |record| match record {
        crate::log_framing::Record::Bytes(bytes) => emit(&sender, stream, &bytes),
        crate::log_framing::Record::Oversized => sender
            .send(Event::Line {
                stream,
                message: OMITTED_RECORD.into(),
            })
            .is_ok(),
    };
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = sender.send(Event::ReadFailed {
                    stream,
                    kind: error.kind(),
                });
                return;
            }
        };
        if available.is_empty() {
            framer.finish(&mut emit_record);
            return;
        }
        if !framer.feed(available, &mut emit_record) {
            return;
        }
        let length = available.len();
        reader.consume(length);
    }
}

#[cfg(test)]
#[path = "installation_logs_tests.rs"]
mod tests;

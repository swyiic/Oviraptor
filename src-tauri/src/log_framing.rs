//! Shared bounded byte framing. Oversized records are discarded whole, never split.
//! Consumers select their own redaction/display policy after a complete record.
#[derive(Debug)]
pub(crate) enum Record {
    Bytes(Vec<u8>),
    Oversized,
}

pub(crate) struct Framer {
    bytes: Vec<u8>,
    limit: usize,
    discarding: bool,
}
impl Framer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit),
            limit,
            discarding: false,
        }
    }
    pub(crate) fn feed(&mut self, input: &[u8], emit: &mut dyn FnMut(Record) -> bool) -> bool {
        for &byte in input {
            if matches!(byte, b'\r' | b'\n') {
                if !self.discarding && !self.bytes.is_empty() {
                    let record = std::mem::replace(&mut self.bytes, Vec::with_capacity(self.limit));
                    if !emit(Record::Bytes(record)) {
                        return false;
                    }
                }
                self.bytes.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.bytes.len() == self.limit {
                    self.bytes.clear();
                    self.discarding = true;
                    if !emit(Record::Oversized) {
                        return false;
                    }
                } else {
                    self.bytes.push(byte);
                }
            }
        }
        true
    }
    pub(crate) fn abandon_pending(&mut self) {
        self.bytes.clear();
        self.discarding = false;
    }
    pub(crate) fn finish(&mut self, emit: &mut dyn FnMut(Record) -> bool) -> bool {
        if self.discarding || self.bytes.is_empty() {
            return true;
        }
        emit(Record::Bytes(std::mem::take(&mut self.bytes)))
    }
}

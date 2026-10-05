//! §13 COR-007 — every resource a bundle can consume is bounded, and going over a
//! bound is a controlled failure with a code, never a partial import.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub file_bytes: u64,
    pub bundle_bytes: u64,
    pub bundle_files: usize,
    pub json_depth: usize,
    pub json_line_bytes: usize,
    /// Bundle record bound, checked before reconciliation and publication.
    pub records: usize,
    pub text_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 32 * 1024 * 1024,
            bundle_bytes: 256 * 1024 * 1024,
            bundle_files: 20_000,
            json_depth: 24,
            json_line_bytes: 4 * 1024 * 1024,
            records: 20_000,
            text_chars: 200_000,
        }
    }
}

impl Limits {
    /// Rejects a document nested deeper than the configured bound before it is fed
    /// to the parser, so a hostile file cannot reach the recursion limit.
    pub fn json_within_depth(&self, bytes: &[u8]) -> bool {
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for character in bytes {
            if in_string {
                if escaped {
                    escaped = false;
                } else if *character == b'\\' {
                    escaped = true;
                } else if *character == b'"' {
                    in_string = false;
                }
                continue;
            }
            match character {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > self.json_depth {
                        return false;
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        true
    }

    pub fn bounded_text(&self, text: &str) -> String {
        if text.chars().count() <= self.text_chars {
            return text.to_string();
        }
        let mut kept: String = text.chars().take(self.text_chars).collect();
        kept.push_str("…[truncated]");
        kept
    }
}

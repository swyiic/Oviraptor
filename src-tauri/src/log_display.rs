//! Shared presentation boundary for log events, stored asset logs and tail reads.
//! Redact before truncation so clipping never exposes a partial credential.
pub(crate) fn text(value: &str, max_chars: usize) -> String {
    let mut plain = String::with_capacity(value.len());
    let mut in_escape = false;
    for character in value.chars() {
        if in_escape {
            if character.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
        } else if character != '\r' {
            plain.push(character);
        }
    }
    crate::agent_runtime::secrets::redact_text_with(plain.trim(), None)
        .chars()
        .take(max_chars)
        .collect()
}

pub(crate) fn line(value: &str) -> String {
    text(value, 1200)
}


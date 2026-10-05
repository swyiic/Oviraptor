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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_log_display_strips_ansi_and_credentials_before_character_limit() {
        let value = line("\x1b[32mFOFA_KEY=private-fofa-key password=private-password\x1b[0m");
        assert!(!value.contains("private-"));
        assert!(!value.contains('\x1b'));
        assert_eq!(line(&"界".repeat(1400)).chars().count(), 1200);
        assert_eq!(text("Bearer private-bearer-token", 12), "Bearer <reda");
    }

    #[test]
    fn shared_log_display_keeps_multiline_failure_structure_and_valid_repetition() {
        assert_eq!(line(" first\r\nfirst\nlast "), "first\nfirst\nlast");
        let value = text("command failed\npassword=private-password\nexit 1", 8000);
        assert!(value.starts_with("command failed\n"));
        assert!(value.ends_with("\nexit 1"));
        assert!(!value.contains("private-password"));
    }
}

//! Compare redaction structure of an already-paid semantic receipt, without
//! reinterpreting a durable one-way marker as a fresh Cookie/password value.
//! The original SDK parse still runs the full redactor on every raw string.
use crate::agent_runtime::secrets;

pub(super) fn verified_safe(text: &str) -> bool {
    canonical_markers(text) == canonical_markers(&secrets::redact_text_with(text, None))
}
fn canonical_markers(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<redacted:") {
        output.push_str(&rest[..start]);
        let tail = &rest[start + "<redacted:".len()..];
        let Some(end) = tail.find('>') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let marker = &tail[..end];
        let valid = marker.split_once(':').filter(|(kind, hash)| {
            matches!(
                *kind,
                "auth" | "assertion" | "bearer" | "opaque" | "email" | "national_id" | "phone"
            ) && hash.len() == 12
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        if let Some((kind, _)) = valid {
            output.push_str(&format!("<redacted:{kind}:000000000000>"));
        } else {
            output.push_str(&rest[start..start + "<redacted:".len() + end + 1]);
        }
        rest = &tail[end + 1..];
    }
    output.push_str(rest);
    output
}

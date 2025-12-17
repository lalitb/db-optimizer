use lazy_static::lazy_static;
use regex::Regex;

/// Redact potentially sensitive literals from SQL for display/logging.
///
/// Intentionally heuristic (not a full SQL parser): it targets common leak vectors
/// like string literals, numeric literals, UUIDs, and hex blobs.
pub fn redact_sql_for_display(sql: &str, max_len: usize) -> String {
    let cleaned = sql
        .trim()
        .replace(['\n', '\r', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    lazy_static! {
        // '...': handles doubled quotes inside strings.
        static ref RE_SINGLE_QUOTED: Regex = Regex::new(r"'(?:''|[^'])*'").unwrap();
        // UUIDs
        static ref RE_UUID: Regex = Regex::new(
            r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b"
        )
        .unwrap();
        // Hex-ish tokens (API keys, digests, etc.)
        static ref RE_HEX_BLOB: Regex = Regex::new(r"\b0x[0-9a-fA-F]{8,}\b|\b[0-9a-fA-F]{32,}\b").unwrap();
        // Numbers (ints/floats). Avoids touching identifiers by requiring word boundaries.
        static ref RE_NUMBER: Regex = Regex::new(r"\b\d+(?:\.\d+)?\b").unwrap();
    }

    let redacted = RE_SINGLE_QUOTED.replace_all(&cleaned, "'<redacted>'");
    let redacted = RE_UUID.replace_all(&redacted, "<uuid>");
    let redacted = RE_HEX_BLOB.replace_all(&redacted, "<redacted>");
    let redacted = RE_NUMBER.replace_all(&redacted, "<n>");

    if redacted.len() <= max_len {
        redacted.into_owned()
    } else {
        format!("{}...", &redacted[..max_len])
    }
}

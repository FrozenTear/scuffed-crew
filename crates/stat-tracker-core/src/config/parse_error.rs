//! config.toml parse errors that are safe to show in Settings and to log.
//!
//! toml's own errors quote the offending line in `Display`, and
//! `toml::de::Error`'s `Debug` carries the whole file. config.toml holds the
//! sync token, so neither reaches the member or a log. What is kept is
//! toml's one-line message with the line and column, and every secret value
//! found in the file is replaced by [`REDACTED`] in that text too (a type
//! error such as "invalid type: integer `123`" quotes the value).

use std::ops::Range;

/// Stands in for a secret value in error text.
pub const REDACTED: &str = "[redacted]";

/// A secret shorter than this is not replaced: it would blank ordinary
/// letters of the message, and the snippet that could quote it is gone.
/// Site tokens are far longer.
const MIN_SECRET_LEN: usize = 4;

/// The two toml error types the config code meets.
pub(super) trait TomlFailure {
    fn message(&self) -> &str;
    fn span(&self) -> Option<Range<usize>>;
}

impl TomlFailure for toml::de::Error {
    fn message(&self) -> &str {
        toml::de::Error::message(self)
    }
    fn span(&self) -> Option<Range<usize>> {
        toml::de::Error::span(self)
    }
}

impl TomlFailure for toml_edit::TomlError {
    fn message(&self) -> &str {
        toml_edit::TomlError::message(self)
    }
    fn span(&self) -> Option<Range<usize>> {
        toml_edit::TomlError::span(self)
    }
}

/// One line about why `source` did not parse: the position and toml's
/// message, never the source line, with secrets from `source` redacted.
pub(super) fn describe(source: &str, err: &impl TomlFailure) -> String {
    let text = match err.span() {
        Some(span) => {
            let (line, column) = line_and_column(source, span.start);
            format!(
                "TOML parse error at line {line}, column {column}: {}",
                err.message().trim()
            )
        }
        None => err.message().trim().to_string(),
    };
    redact_secrets(source, &redact_quoted_values(&text))
}

/// Where serde quotes the value it rejected, with the closing delimiter:
/// `invalid type: string "…"`, `unknown variant `…``, and the number and
/// char forms. A token pasted under any key (`sync = "<token>"`,
/// `debug_ocr = "<token>"`) comes back this way, whatever the key is called,
/// so these payloads are blanked without looking at the key. The type and
/// field wording around them stays.
const QUOTED_VALUE_MARKERS: [(&str, char); 6] = [
    ("string \"", '"'),
    ("variant `", '`'),
    ("integer `", '`'),
    ("floating point `", '`'),
    ("char '", '\''),
    ("bytes \"", '"'),
];

/// `text` with every payload after a [`QUOTED_VALUE_MARKERS`] marker
/// replaced by [`REDACTED`]. A backslash escapes the next character (serde
/// writes strings with `{:?}`), and a payload with no closing delimiter
/// runs to the end of the text.
pub(super) fn redact_quoted_values(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        let next = QUOTED_VALUE_MARKERS
            .iter()
            .filter_map(|(marker, close)| rest.find(marker).map(|at| (at, *marker, *close)))
            .min_by_key(|(at, marker, _)| (*at, std::cmp::Reverse(marker.len())));
        let Some((at, marker, close)) = next else {
            break;
        };
        let body_start = at + marker.len();
        out.push_str(&rest[..body_start]);
        let body = &rest[body_start..];
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            if c == '\\' {
                chars.next();
            } else if c == close {
                out.push_str(REDACTED);
                rest = &body[i..];
                continue 'scan;
            }
        }
        out.push_str(REDACTED);
        rest = "";
    }
    out.push_str(rest);
    out
}

/// 1-based line and column (in characters) of byte `index` in `source`.
fn line_and_column(source: &str, index: usize) -> (usize, usize) {
    let mut index = index.min(source.len());
    while !source.is_char_boundary(index) {
        index -= 1;
    }
    let before = &source[..index];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |nl| nl + 1);
    let column = source[line_start..index].chars().count() + 1;
    (line, column)
}

/// `text` with every secret value written in `source` replaced. `source`
/// need not parse: values are found line by line.
pub fn redact_secrets(source: &str, text: &str) -> String {
    let mut secrets = secret_values(source);
    // Longest first, so a secret that contains another is replaced whole.
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    let mut out = text.to_string();
    for secret in &secrets {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), REDACTED);
        }
    }
    out
}

/// Values of secret-bearing keys, inline tables and dotted keys included.
fn secret_values(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        for (eq, _) in line.match_indices('=') {
            if !is_secret_key(key_before(&line[..eq])) {
                continue;
            }
            for value in value_after(&line[eq + 1..]) {
                if value.chars().count() >= MIN_SECRET_LEN {
                    out.push(value);
                }
            }
        }
    }
    out
}

/// `token`, `*_token`, `*-token`, and anything naming a secret or password.
/// `[sync] token` is the one config.toml has today.
fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "token"
        || key.ends_with("_token")
        || key.ends_with("-token")
        || key.contains("secret")
        || key.contains("password")
}

/// The last segment of the key that ends right before an `=`.
fn key_before(before: &str) -> &str {
    let before = before.trim_end();
    if let Some(quote) = before.chars().last().filter(|c| *c == '"' || *c == '\'') {
        let inner = &before[..before.len() - 1];
        return match inner.rfind(quote) {
            Some(open) => &inner[open + 1..],
            None => inner,
        };
    }
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
        .map_or(0, |(i, c)| i + c.len_utf8());
    &before[start..]
}

/// The value written after an `=`, as raw text, plus its unescaped form
/// for a basic string. An unterminated string runs to the end of the line.
///
/// The unescaping is rough (`\u0041` comes out as `u0041`, `\t` as `t`), and
/// a `"""` value that starts on the next line is not found. Neither matters
/// for the site's tokens, which are 64 hex characters, and serde's quoted
/// payloads are blanked by [`redact_quoted_values`] whatever this finds. Do
/// not rely on this function for values with escapes.
fn value_after(after: &str) -> Vec<String> {
    let rest = after.trim_start();
    let mut out = Vec::new();
    for delim in ["\"\"\"", "'''"] {
        if let Some(body) = rest.strip_prefix(delim) {
            let end = body.find(delim).unwrap_or(body.len());
            push_trimmed(&mut out, &body[..end]);
            return out;
        }
    }
    if let Some(body) = rest.strip_prefix('"') {
        let mut raw_end = body.len();
        let mut unescaped = String::new();
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    raw_end = i;
                    break;
                }
                '\\' => {
                    if let Some((_, next)) = chars.next() {
                        unescaped.push(next);
                    }
                }
                c => unescaped.push(c),
            }
        }
        push_trimmed(&mut out, &body[..raw_end]);
        push_trimmed(&mut out, &unescaped);
        return out;
    }
    if let Some(body) = rest.strip_prefix('\'') {
        let end = body.find('\'').unwrap_or(body.len());
        push_trimmed(&mut out, &body[..end]);
        return out;
    }
    let end = rest.find([',', '}', ']', '#']).unwrap_or(rest.len());
    push_trimmed(&mut out, &rest[..end]);
    out
}

fn push_trimmed(out: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !out.iter().any(|seen| seen == value) {
        out.push(value.to_string());
    }
}

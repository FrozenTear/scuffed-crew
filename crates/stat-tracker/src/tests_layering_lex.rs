//! The lexer helpers of [`super`]: blank out comments and literals, and
//! find brackets and item ends in the blanked text.

pub(super) fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text` with comments, string literals and char literals replaced by
/// spaces. Newlines stay, so offsets and line numbers still match.
pub(super) fn blank(text: &str) -> Vec<char> {
    let src: Vec<char> = text.chars().collect();
    let mut out = src.clone();
    let n = src.len();
    let wipe = |out: &mut Vec<char>, a: usize, b: usize| {
        for c in out.iter_mut().take(b.min(n)).skip(a) {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let mut i = 0;
    while i < n {
        let c = src[i];
        let next = src.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            let end = (i..n).find(|&k| src[k] == '\n').unwrap_or(n);
            wipe(&mut out, i, end);
            i = end;
        } else if c == '/' && next == Some('*') {
            let (mut depth, mut k) = (1, i + 2);
            while k < n && depth > 0 {
                if src[k] == '/' && src.get(k + 1) == Some(&'*') {
                    depth += 1;
                    k += 2;
                } else if src[k] == '*' && src.get(k + 1) == Some(&'/') {
                    depth -= 1;
                    k += 2;
                } else {
                    k += 1;
                }
            }
            wipe(&mut out, i, k);
            i = k;
        } else if let Some(end) = string_end(&src, i) {
            wipe(&mut out, i, end);
            i = end;
        } else if c == '\'' {
            // A char literal ('x', '\n', '\u{..}'), or a lifetime / label.
            let end = if next == Some('\\') {
                // the escaped char may itself be a quote: '\''
                (i + 3..n).find(|&k| src[k] == '\'').map(|k| k + 1)
            } else if src.get(i + 2) == Some(&'\'') {
                Some(i + 3)
            } else {
                None
            };
            match end {
                Some(end) => {
                    wipe(&mut out, i, end);
                    i = end;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The end of the string literal starting at `i` (`"…"`, `b"…"`, `c"…"`,
/// `r#"…"#`, `br"…"`), or `None` when none starts there.
fn string_end(src: &[char], i: usize) -> Option<usize> {
    if i > 0 && is_ident(src[i - 1]) {
        return None;
    }
    let mut k = i;
    if matches!(src.get(k), Some('b') | Some('c')) {
        k += 1;
    }
    let raw = src.get(k) == Some(&'r');
    if raw {
        k += 1;
    }
    let hashes = src[k..].iter().take_while(|c| **c == '#').count();
    k += hashes;
    if src.get(k) != Some(&'"') || (!raw && hashes > 0) {
        return None;
    }
    k += 1;
    while k < src.len() {
        if raw {
            if src[k] == '"'
                && src[k + 1..]
                    .iter()
                    .take(hashes)
                    .filter(|c| **c == '#')
                    .count()
                    == hashes
            {
                return Some(k + 1 + hashes);
            }
            k += 1;
        } else if src[k] == '\\' {
            k += 2;
        } else if src[k] == '"' {
            return Some(k + 1);
        } else {
            k += 1;
        }
    }
    Some(src.len())
}

/// Index of the bracket that closes the one at `open`.
pub(super) fn closing(t: &[char], open: usize) -> usize {
    let (o, c) = match t[open] {
        '{' => ('{', '}'),
        '(' => ('(', ')'),
        _ => ('[', ']'),
    };
    let mut depth = 0usize;
    for (k, ch) in t.iter().enumerate().skip(open) {
        if *ch == o {
            depth += 1;
        } else if *ch == c {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    t.len().saturating_sub(1)
}

pub(super) fn starts_with(t: &[char], at: usize, s: &str) -> bool {
    s.chars()
        .enumerate()
        .all(|(k, c)| t.get(at + k) == Some(&c))
}

pub(super) fn skip_ws(t: &[char], mut k: usize) -> usize {
    while k < t.len() && t[k].is_whitespace() {
        k += 1;
    }
    k
}

/// The end of the item that starts at `k`: its `;` or its closing brace.
pub(super) fn item_end(t: &[char], mut k: usize) -> usize {
    let mut depth = 0i32;
    while k < t.len() {
        match t[k] {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ';' if depth <= 0 => return k,
            '{' if depth <= 0 => return closing(t, k),
            _ => {}
        }
        k += 1;
    }
    t.len()
}

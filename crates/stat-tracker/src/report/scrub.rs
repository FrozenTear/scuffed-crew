//! Log scrub for report bundles (report-bundle spec section 6).
//!
//! Pure functions. [`ScrubContext`] holds the secrets and names to remove,
//! [`scrub_text`] replaces them with the section 6 placeholders, and
//! [`scrub_log`] keeps only the timestamped lines inside a [`LogWindow`],
//! drops reader name fields, and scrubs what is left.
//!
//! Every rule finds its matches on the original text. Matches are then taken
//! longest first. A shorter match that overlaps a longer one is merged into
//! it, so no part of a matched secret survives beside a placeholder.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Utc};
use regex::Regex;

pub const PH_SYNC_TOKEN: &str = "[sync-token]";
pub const PH_SERVER_URL: &str = "[server-url]";
pub const PH_HOME: &str = "[home]";
pub const PH_USER: &str = "[user]";
pub const PH_HOST: &str = "[host]";
pub const PH_BATTLETAG: &str = "[battletag]";
pub const PH_PLAYER: &str = "[player]";

/// `log.txt` body when the window holds no line.
pub const NO_LINES_IN_WINDOW: &str = "[no log lines in window]";

/// Most bytes of scrubbed log kept in `log.txt`. The newest lines win. The
/// whole bundle must stay under 10 MB uncompressed (section 8).
pub const LOG_MAX_BYTES: usize = 4 * 1024 * 1024;

/// First log line of `log.txt` when older lines were cut to fit
/// [`LOG_MAX_BYTES`]. Only the report's `host` line comes before it.
pub const LOG_CUT_NOTE: &str = "[older log lines cut to fit the size cap]";

/// Log field keys whose value is a player name. The field is dropped from
/// `log.txt`, key and value, not only scrubbed.
pub const NAME_FIELD_KEYS: &[&str] = &[
    "name",
    "names",
    "player",
    "players",
    "player_name",
    "self_name",
    "nick",
    "nickname",
    "battletag",
    "row_name",
];

/// Log field keys whose value is raw OCR text. The daemon logs these on
/// lines that are not about names, but the text can hold player names: the
/// full-frame Tab read (`text_preview`, mostly the name column), the POTG
/// and end-title reads (`text`, `ocr_text`), and the accolade screen
/// (`raw`). There is no game in a log-only report, so no name list exists
/// to scrub them with. They are always dropped, key and value.
pub const OCR_TEXT_KEYS: &[&str] = &[
    "text",
    "text_preview",
    "ocr_text",
    "raw",
    "read",
    "cell",
    "cell_text",
];

/// Values the scrub removes. Empty strings and `None` are ignored.
#[derive(Debug, Clone, Default)]
pub struct ScrubContext {
    /// Configured sync token, removed on exact match.
    pub sync_token: Option<String>,
    /// Configured server URL. Any URL or bare mention of its host is
    /// removed too, with any port. The `www.` and bare forms of the host
    /// count as the same site, and so does any name under it.
    pub server_url: Option<String>,
    /// Home directory of the account running the app.
    pub home_dir: Option<PathBuf>,
    /// Account (login) name.
    pub account_name: Option<String>,
    /// Machine hostname.
    pub hostname: Option<String>,
    /// Configured player name, with or without the `#` discriminator.
    pub player_name: Option<String>,
    /// Every other name read in the window: other players, misreads, and
    /// short names of one or two characters.
    pub names: Vec<String>,
}

impl ScrubContext {
    /// Context for this machine and this config: sync token and URL, player
    /// name, home directory, account name, and hostname. `names` starts
    /// empty; the caller adds the names read in the window.
    pub fn from_config(config: &crate::config::Config) -> Self {
        let home_dir = dirs::home_dir();
        let account_name = std::env::var("USER")
            .ok()
            .or_else(|| std::env::var("LOGNAME").ok())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                home_dir
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned())
            });
        Self {
            sync_token: config.sync.as_ref().map(|s| s.token.clone()),
            server_url: config.sync.as_ref().map(|s| s.server_url.clone()),
            home_dir,
            account_name,
            hostname: local_hostname(),
            player_name: config.player_name.clone(),
            names: Vec::new(),
        }
    }
}

/// Kernel hostname, or `None` when it cannot be read.
pub fn local_hostname() -> Option<String> {
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Time ranges whose lines go into `log.txt`. Bounds are inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogWindow {
    ranges: Vec<(DateTime<Utc>, DateTime<Utc>)>,
}

impl LogWindow {
    /// Game report: 120 seconds before the first stored capture to 120
    /// seconds after the last one.
    pub fn session(first_capture: DateTime<Utc>, last_capture: DateTime<Utc>) -> Self {
        let pad = Duration::seconds(120);
        let (a, b) = if first_capture <= last_capture {
            (first_capture, last_capture)
        } else {
            (last_capture, first_capture)
        };
        Self {
            ranges: vec![(a - pad, b + pad)],
        }
    }

    /// `performance` or `other` report, or a `crash` report with no crash
    /// marker: the 30 minutes before the report is built.
    pub fn log_only(built_at: DateTime<Utc>) -> Self {
        Self {
            ranges: vec![(built_at - Duration::minutes(30), built_at)],
        }
    }

    /// `crash` report with a crash marker: 30 minutes before the marker's
    /// `last_seen` through 2 minutes after it, plus the 30 minutes before
    /// the build. When the two overlap, a line in either is kept (their
    /// union). The marker's start time is not an edge.
    pub fn crash(built_at: DateTime<Utc>, last_seen: DateTime<Utc>) -> Self {
        let mut w = Self::log_only(built_at);
        w.ranges.insert(
            0,
            (
                last_seen - Duration::minutes(30),
                last_seen + Duration::minutes(2),
            ),
        );
        w
    }

    /// Section 6 window for a log-only report built at `built_at`.
    /// `crash_last_seen` is the crash marker's `last_seen`, used only for a
    /// `crash` report.
    pub fn for_log_only(
        built_at: DateTime<Utc>,
        is_crash: bool,
        crash_last_seen: Option<DateTime<Utc>>,
    ) -> Self {
        match crash_last_seen {
            Some(seen) if is_crash => Self::crash(built_at, seen),
            _ => Self::log_only(built_at),
        }
    }

    pub fn contains(&self, t: DateTime<Utc>) -> bool {
        self.ranges.iter().any(|(a, b)| *a <= t && t <= *b)
    }

    pub fn ranges(&self) -> &[(DateTime<Utc>, DateTime<Utc>)] {
        &self.ranges
    }

    /// Earliest instant the window keeps, or `None` for an empty window.
    pub fn start(&self) -> Option<DateTime<Utc>> {
        self.ranges.iter().map(|(a, _)| *a).min()
    }

    /// Latest instant the window keeps, or `None` for an empty window.
    pub fn end(&self) -> Option<DateTime<Utc>> {
        self.ranges.iter().map(|(_, b)| *b).max()
    }
}

static ANSI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("ansi regex"));
static BEARER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bbearer\s+([^\s,;]+)").expect("bearer regex"));
static TOKEN_RUN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9_.\-]{24,}").expect("token regex"));
static BASE64_RUN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/]{24,}={0,2}").expect("base64 regex"));
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\b[a-z][a-z0-9+.\-]*://[^\s"'<>]+"#).expect("url regex"));
/// Characters cut from the end of a URL match before its host is read.
const URL_TRAILING_PUNCT: &[char] = &['.', ',', ';', ':', '!', '?', ')', '>', '"', '\''];
/// `scheme://` at the end of the [`HostView`] before a host.
static SCHEME_SEP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b[a-z][a-z0-9+.\-]*://\z").expect("scheme regex"));
static HOME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"/(?:var/home|home|Users)/[^/\s"'=,;:]+"#).expect("home regex"));
// Wider than the 2..12 letters a BattleTag allows: a longer run before `#`
// would otherwise leave its first letters behind beside the placeholder.
static BATTLETAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{M}\p{N}_]{2,}#[0-9]{4,8}").expect("battletag regex"));
static FIELD_START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)([A-Za-z_][A-Za-z0-9_.]*)=").expect("field regex"));

#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    placeholder: &'static str,
}

/// Replace every section 6 value in `text` with its placeholder.
pub fn scrub_text(ctx: &ScrubContext, text: &str) -> String {
    Scrubber::new(ctx).scrub(text)
}

/// One literal rule, compiled once per bundle.
struct Literal {
    re: Regex,
    whole_token: bool,
    placeholder: &'static str,
}

/// [`ScrubContext`] with its literal patterns compiled.
pub struct Scrubber {
    literals: Vec<Literal>,
    site: Option<SiteHost>,
}

/// The configured site host, matched in its `www.` and bare forms.
struct SiteHost {
    /// Lowercase host without a leading `www.` or a trailing dot, in its
    /// punycode and Unicode forms (one entry when they are the same).
    apexes: Vec<String>,
    /// Each apex, case-insensitive, matched on the [`HostView`] of the
    /// text. Zero-width characters may sit between any two of its
    /// characters.
    apex_res: Vec<Regex>,
}

impl SiteHost {
    fn new(host: &str) -> Option<Self> {
        let host = host.strip_suffix('.').unwrap_or(host).to_lowercase();
        if !host.chars().any(char::is_alphanumeric) {
            return None;
        }
        // `www.example` keeps its `www.`: a single label is not a site.
        let apex = match host.strip_prefix("www.") {
            Some(rest) if rest.contains('.') => rest.to_string(),
            _ => host,
        };
        let mut apexes = vec![apex.clone()];
        // The sync client logs hosts in punycode (`xn--…`); a person may
        // type either form in Settings.
        if let Ok(ascii) = idna::domain_to_ascii(&apex) {
            apexes.push(ascii);
        }
        let (unicode, ok) = idna::domain_to_unicode(&apex);
        if ok.is_ok() {
            apexes.push(unicode.to_lowercase());
        }
        apexes.retain(|a| !a.is_empty());
        apexes.sort();
        apexes.dedup();
        let apex_res = apexes
            .iter()
            .filter_map(|a| Regex::new(&format!("(?i){}", apex_pattern(a))).ok())
            .collect();
        Some(Self { apexes, apex_res })
    }

    /// `host` is the site: an apex itself or a name under it.
    fn is_site(&self, host: &str) -> bool {
        let host = host.strip_suffix('.').unwrap_or(host).to_lowercase();
        self.apexes.iter().any(|apex| {
            host == *apex
                || host
                    .strip_suffix(apex.as_str())
                    .is_some_and(|front| front.ends_with('.'))
        })
    }

    /// Bare mentions of the site host, read on the [`HostView`] of the
    /// text. Each apex occurrence is checked on its own, so a rejected
    /// longer candidate never hides a clean apex:
    /// - Labels in front (`www.`, `api.`, `bücher.`) and a port behind are
    ///   taken in when they are clean, else the plain apex is scrubbed.
    /// - The apex must not sit inside a longer host name: `notcrew.example`,
    ///   `crew.example.evil.com` and `crew.example-evil.com` stay. Only an
    ///   ASCII host character in front rejects it, so `.crew.example`,
    ///   `...crew.example` and `ü.crew.example` are scrubbed.
    /// - A percent escape counts as the character it encodes, even when it
    ///   is encoded again (`%252E`), so `https%3A%2F%2Fwww%2Ecrew.example%3A8443`
    ///   and `%63rew.example` are scrubbed whole.
    /// - An IDNA full stop (`。`) counts as a dot between labels, and a
    ///   zero-width character inside the host is skipped.
    fn push_bare(&self, view: &HostView<'_>, out: &mut Vec<Span>) {
        let dec = view.dec.as_ref();
        for re in &self.apex_res {
            // End of the last match taken: the next label walk stops there.
            let mut floor = 0;
            for m in re.find_iter(dec) {
                if m.start() < floor {
                    continue;
                }
                if dec[..m.start()]
                    .chars()
                    .next_back()
                    .is_some_and(is_ascii_host_char)
                {
                    continue;
                }
                if continues_host(view, m.end()) {
                    continue;
                }
                let start = labels_start(dec, floor, m.start());
                let start = encoded_scheme_start(view, floor, start);
                let end = m.end() + port_len(view, m.end());
                floor = end;
                out.push(Span {
                    start: view.orig(start),
                    end: view.orig(end),
                    placeholder: PH_SERVER_URL,
                });
            }
        }
    }
}

/// Characters IDNA drops from a host name: zero-width space, zero-width
/// non-joiner and joiner, the byte-order mark and the soft hyphen. They are
/// skipped inside the site host. Right before or after it they still end
/// it, as before.
fn is_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{AD}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'
    )
}

/// Any run of [`is_ignorable`] characters, as a regex.
const IGNORABLE_RUN: &str = r"[\x{AD}\x{200B}-\x{200D}\x{FEFF}]*";

/// IDNA full stops that count as a dot between labels: ideographic (`。`),
/// fullwidth (`．`) and halfwidth (`｡`).
fn is_idna_dot(c: char) -> bool {
    matches!(c, '\u{3002}' | '\u{FF0E}' | '\u{FF61}')
}

/// Regex for one apex: its characters with any zero-width run between
/// them.
fn apex_pattern(apex: &str) -> String {
    let mut buf = [0u8; 4];
    apex.chars()
        .map(|c| regex::escape(c.encode_utf8(&mut buf)))
        .collect::<Vec<_>>()
        .join(IGNORABLE_RUN)
}

/// Most layers of percent-encoding read: `%2E`, `%252E` and `%25252E` are
/// all a dot.
const MAX_ESCAPE_LAYERS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitKind {
    /// A percent escape, maybe encoded again.
    Escape,
    /// An IDNA full stop, read as `.`.
    IdnaDot,
}

/// One character of a [`HostView`] that is not a plain copy of the text.
#[derive(Debug, Clone, Copy)]
struct Unit {
    /// Byte offset in the view, and the character's length there.
    dec: usize,
    len: usize,
    /// End of its bytes in the text.
    end: usize,
    kind: UnitKind,
}

/// The text as the bare host rule reads it. Each percent escape is decoded
/// to the character it stands for, up to [`MAX_ESCAPE_LAYERS`] deep (a byte
/// above ASCII becomes U+FFFD, which no host holds), and each IDNA full
/// stop is read as `.`. Everything else is copied, so offsets map back to
/// the text with one binary search.
struct HostView<'a> {
    dec: Cow<'a, str>,
    /// Every character that is not a plain copy, in text order.
    units: Vec<Unit>,
}

impl<'a> HostView<'a> {
    fn new(text: &'a str) -> Self {
        let b = text.as_bytes();
        let mut dec = String::new();
        let mut units = Vec::new();
        let (mut i, mut copied) = (0, 0);
        while i < b.len() {
            let found = match b[i] {
                b'%' => decode_escape(&b[i..]).map(|(c, n)| (c, n, UnitKind::Escape)),
                x if x >= 0x80 => {
                    let c = text[i..].chars().next().unwrap_or('\u{FFFD}');
                    if !is_idna_dot(c) {
                        i += c.len_utf8();
                        continue;
                    }
                    Some(('.', c.len_utf8(), UnitKind::IdnaDot))
                }
                _ => None,
            };
            let Some((c, n, kind)) = found else {
                i += 1;
                continue;
            };
            dec.push_str(&text[copied..i]);
            units.push(Unit {
                dec: dec.len(),
                len: c.len_utf8(),
                end: i + n,
                kind,
            });
            dec.push(c);
            i += n;
            copied = i;
        }
        if units.is_empty() {
            return Self {
                dec: Cow::Borrowed(text),
                units,
            };
        }
        dec.push_str(&text[copied..]);
        Self {
            dec: Cow::Owned(dec),
            units,
        }
    }

    /// Byte offset in the text of view offset `at` (a char boundary).
    fn orig(&self, at: usize) -> usize {
        let before = self.units.partition_point(|u| u.dec + u.len <= at);
        match before.checked_sub(1).map(|i| self.units[i]) {
            Some(u) => u.end + (at - (u.dec + u.len)),
            None => at,
        }
    }

    /// How the character at view offset `at` was read, or `None` for a
    /// plain copy.
    fn kind_at(&self, at: usize) -> Option<UnitKind> {
        let i = self.units.partition_point(|u| u.dec < at);
        self.units.get(i).filter(|u| u.dec == at).map(|u| u.kind)
    }
}

/// The character a percent escape at the start of `b` stands for, and the
/// escape's length in bytes. After an encoded `%` (`%25`), two more hex
/// digits are read again, up to [`MAX_ESCAPE_LAYERS`] in all.
fn decode_escape(b: &[u8]) -> Option<(char, usize)> {
    let hex = |at: usize| match b.get(at..at + 2) {
        Some(&[h, l]) if h.is_ascii_hexdigit() && l.is_ascii_hexdigit() => {
            let digit = |d: u8| char::from(d).to_digit(16).unwrap_or(0) as u8;
            Some(digit(h) << 4 | digit(l))
        }
        _ => None,
    };
    if b.first() != Some(&b'%') {
        return None;
    }
    let mut v = hex(1)?;
    let mut n = 3;
    let mut layers = 1;
    while v == b'%' && layers < MAX_ESCAPE_LAYERS {
        let Some(w) = hex(n) else {
            break;
        };
        v = w;
        n += 2;
        layers += 1;
    }
    let c = if v.is_ascii() {
        char::from(v)
    } else {
        '\u{FFFD}'
    };
    Some((c, n))
}

/// The site host goes on at view offset `at`: a host character, or a dot
/// and then one (`crew.example.evil.com`). A trailing sentence dot does not
/// count, and neither does an IDNA full stop: after a host, `。` is the CJK
/// end of a sentence.
fn continues_host(view: &HostView<'_>, at: usize) -> bool {
    let mut rest = view.dec[at..].chars();
    match rest.next() {
        Some(c) if is_ascii_host_char(c) => true,
        Some('.') if view.kind_at(at) != Some(UnitKind::IdnaDot) => {
            rest.next().is_some_and(is_ascii_host_char)
        }
        _ => false,
    }
}

/// Bytes of a `:port` at view offset `at`, or 0. A port that runs on into
/// a host character (`:8443x`) is left out, and only the host is scrubbed.
/// An encoded `%3A` counts as the `:`.
fn port_len(view: &HostView<'_>, at: usize) -> usize {
    let Some(digits) = view.dec[at..].strip_prefix(':') else {
        return 0;
    };
    let n = digits.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 || continues_host(view, at + 1 + n) {
        return 0;
    }
    1 + n
}

/// Longest host name in front of an apex that the label walk looks at,
/// in bytes: the DNS name limit.
const MAX_HOST_BYTES: usize = 253;

/// `floor`, raised to at most [`MAX_HOST_BYTES`] before `at` and then to a
/// char boundary.
fn walk_floor(text: &str, floor: usize, at: usize) -> usize {
    let mut floor = floor.max(at.saturating_sub(MAX_HOST_BYTES));
    while floor < at && !text.is_char_boundary(floor) {
        floor += 1;
    }
    floor.min(at)
}

/// Start of the host labels in front of an apex at `at` (`www.`, `api.`,
/// `bücher.`), or `at` when none are clean. `text` is the view, so `%2E`
/// and `。` read as dots. The walk never goes below `floor` (the end of the
/// previous match) or more than [`MAX_HOST_BYTES`] back, so a line of many
/// mentions stays linear.
fn labels_start(text: &str, floor: usize, mut at: usize) -> usize {
    let floor = walk_floor(text, floor, at);
    loop {
        let Some(dot) = text[floor..at].strip_suffix('.') else {
            return at;
        };
        let label_end = floor + dot.len();
        let run: usize = dot
            .chars()
            .rev()
            .take_while(|&c| is_host_char(c) || is_ignorable(c))
            .map(char::len_utf8)
            .sum();
        let label = label_end - run;
        // A label needs a host character. A run right after a `%` that is
        // not an escape is not taken.
        if !text[label..label_end].chars().any(is_host_char) || text[..label].ends_with('%') {
            return at;
        }
        at = label;
    }
}

/// Start of an encoded `scheme%3A%2F%2F` (encoded once or more) right
/// before view offset `at` and not below `floor`, or `at`. A plain
/// `scheme://` is left to the URL rule.
fn encoded_scheme_start(view: &HostView<'_>, floor: usize, at: usize) -> usize {
    let dec = view.dec.as_ref();
    let floor = walk_floor(dec, floor, at);
    match SCHEME_SEP_RE.find(&dec[floor..at]) {
        Some(m) if (at - 3..at).any(|d| view.kind_at(d) == Some(UnitKind::Escape)) => {
            floor + m.start()
        }
        _ => at,
    }
}

fn is_host_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

fn is_ascii_host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Cut sentence punctuation and closing wrappers from the end of a URL
/// match. A `]` closes an IPv6 host when the URL opened one, so it stays.
fn trim_url_end(url: &str) -> &str {
    let mut url = url;
    loop {
        let Some(last) = url.chars().next_back() else {
            return url;
        };
        let cut = match last {
            ']' => url.matches('[').count() < url.matches(']').count(),
            '}' => url.matches('{').count() < url.matches('}').count(),
            c => URL_TRAILING_PUNCT.contains(&c),
        };
        if !cut {
            return url;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
}

impl Scrubber {
    pub fn new(ctx: &ScrubContext) -> Self {
        let mut literals = Vec::new();
        let mut add = |needle: &str, ignore_case: bool, whole_token: bool, ph: &'static str| {
            let needle = needle.trim();
            if needle.is_empty() {
                return;
            }
            let pattern = format!(
                "{}{}",
                if ignore_case { "(?i)" } else { "" },
                regex::escape(needle)
            );
            if let Ok(re) = Regex::new(&pattern) {
                literals.push(Literal {
                    re,
                    whole_token,
                    placeholder: ph,
                });
            }
        };
        if let Some(tok) = non_empty(ctx.sync_token.as_deref()) {
            add(tok, false, false, PH_SYNC_TOKEN);
        }
        // The URL and bare-host rules in `scrub` cover the configured URL
        // on host boundaries. The literal is only a fallback for an address
        // that is not a URL at all. An address with a scheme but no host
        // (`https://`, `http://:8443`) scrubs nothing: its literal would
        // hide every `https://` in the log.
        let server_url = non_empty(ctx.server_url.as_deref());
        let site = server_url
            .and_then(|u| url_host(u).or_else(|| bare_host(u)))
            .and_then(|h| SiteHost::new(&h));
        if let (Some(url), None) = (server_url, &site)
            && !url.contains("://")
        {
            add(url, true, false, PH_SERVER_URL);
        }
        if let Some(home) = ctx.home_dir.as_deref() {
            let home = home.to_string_lossy();
            let home = home.trim_end_matches('/');
            if home.len() > 1 {
                add(home, false, false, PH_HOME);
            }
        }
        if let Some(user) = non_empty(ctx.account_name.as_deref()) {
            add(user, true, true, PH_USER);
        }
        if let Some(host) = non_empty(ctx.hostname.as_deref()) {
            add(host, true, true, PH_HOST);
        }
        let mut names: Vec<&str> = ctx.names.iter().map(String::as_str).collect();
        if let Some(player) = non_empty(ctx.player_name.as_deref()) {
            names.push(player);
            // The configured name is scrubbed without its discriminator too.
            if let Some((bare, _)) = player.split_once('#') {
                names.push(bare);
            }
        }
        for name in names {
            add(name, true, true, PH_PLAYER);
        }
        Self { literals, site }
    }

    pub fn scrub(&self, text: &str) -> String {
        let mut spans = Vec::new();
        let push_re = |re: &Regex, ph: &'static str, out: &mut Vec<Span>| {
            for m in re.find_iter(text) {
                out.push(Span {
                    start: m.start(),
                    end: m.end(),
                    placeholder: ph,
                });
            }
        };
        // Only the credential after the word is replaced.
        for caps in BEARER_RE.captures_iter(text) {
            let m = caps.get(1).expect("credential group");
            spans.push(Span {
                start: m.start(),
                end: m.end(),
                placeholder: PH_SYNC_TOKEN,
            });
        }
        push_re(&TOKEN_RUN_RE, PH_SYNC_TOKEN, &mut spans);
        push_re(&BASE64_RUN_RE, PH_SYNC_TOKEN, &mut spans);
        if let Some(site) = &self.site {
            for m in URL_RE.find_iter(text) {
                // Sentence punctuation after a URL is not part of it.
                let url = trim_url_end(m.as_str());
                if url_host(url).is_some_and(|h| site.is_site(&h)) {
                    spans.push(Span {
                        start: m.start(),
                        end: m.start() + url.len(),
                        placeholder: PH_SERVER_URL,
                    });
                }
            }
            site.push_bare(&HostView::new(text), &mut spans);
        }
        push_re(&HOME_RE, PH_HOME, &mut spans);
        push_re(&BATTLETAG_RE, PH_BATTLETAG, &mut spans);
        for lit in &self.literals {
            push_literal(text, lit, &mut spans);
        }
        apply_spans(text, spans)
    }
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every occurrence of the literal. With `whole_token`, the characters on
/// each side must not be letters, digits, or underscores.
fn push_literal(text: &str, lit: &Literal, out: &mut Vec<Span>) {
    for m in lit.re.find_iter(text) {
        if lit.whole_token {
            let before = text[..m.start()].chars().next_back();
            let after = text[m.end()..].chars().next();
            let first = m.as_str().chars().next();
            let last = m.as_str().chars().next_back();
            // A boundary only matters next to a word character in the match.
            if first.is_some_and(is_word_char) && before.is_some_and(is_word_char) {
                continue;
            }
            if last.is_some_and(is_word_char) && after.is_some_and(is_word_char) {
                continue;
            }
        }
        out.push(Span {
            start: m.start(),
            end: m.end(),
            placeholder: lit.placeholder,
        });
    }
}

/// Overlapping matches merge into one span; matches that only touch stay
/// apart. A merged span keeps the placeholder of its longest match, the
/// first one on a tie. One sort and one pass, so many matches stay fast.
fn apply_spans(text: &str, mut spans: Vec<Span>) -> String {
    spans.retain(|s| s.start < s.end);
    // Stable: matches with the same start keep their rule order.
    spans.sort_by_key(|s| s.start);
    let mut merged: Vec<Span> = Vec::new();
    let mut longest = 0;
    for s in spans {
        match merged.last_mut() {
            Some(cur) if s.start < cur.end => {
                cur.end = cur.end.max(s.end);
                if s.end - s.start > longest {
                    longest = s.end - s.start;
                    cur.placeholder = s.placeholder;
                }
            }
            _ => {
                longest = s.end - s.start;
                merged.push(s);
            }
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for s in merged {
        out.push_str(&text[at..s.start]);
        out.push_str(s.placeholder);
        at = s.end;
    }
    out.push_str(&text[at..]);
    out
}

/// Host of a server address given without `http://` or `https://`, such
/// as `crew.example:8443`. `None` when it holds anything else.
fn bare_host(addr: &str) -> Option<String> {
    let addr = host_chars(addr);
    if addr.contains("://") {
        return None;
    }
    addr.chars()
        .all(|c| is_host_char(c) || matches!(c, '.' | ':' | '/'))
        .then(|| url_host(&format!("https://{addr}")))
        .flatten()
}

/// `s` with each IDNA full stop read as `.` and zero-width characters
/// dropped, as IDNA maps them in a host name.
fn host_chars(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| is_ignorable(c) || is_idna_dot(c)) {
        return Cow::Borrowed(s);
    }
    s.chars()
        .filter(|&c| !is_ignorable(c))
        .map(|c| if is_idna_dot(c) { '.' } else { c })
        .collect()
}

/// Lowercase host of `url`, without port or user info. An IDNA full stop
/// counts as a dot and a zero-width character is skipped, so a site typed
/// as `https://crëw。example` reads as `crëw.example`.
fn url_host(url: &str) -> Option<String> {
    let url = host_chars(url);
    let rest = url.trim().split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        // The host ends at the port or at the first character a host name
        // cannot hold.
        authority
            .split(|c: char| !(is_host_char(c) || c == '.'))
            .next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Remove terminal colour codes the log writer adds.
pub fn strip_ansi(line: &str) -> std::borrow::Cow<'_, str> {
    ANSI_RE.replace_all(line, "")
}

/// Leading RFC 3339 timestamp of a daemon log line.
pub fn line_timestamp(line: &str) -> Option<DateTime<Utc>> {
    let first = line.split_whitespace().next()?;
    DateTime::parse_from_rfc3339(first)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Drop name fields (key and value) from one log line.
pub fn drop_name_fields(line: &str) -> String {
    let mut fields: Vec<(usize, usize, bool)> = Vec::new(); // (start, value_start, drop)
    for caps in FIELD_START_RE.captures_iter(line) {
        let key = caps.get(1).expect("key group");
        let whole = caps.get(0).expect("whole");
        let drop = NAME_FIELD_KEYS.contains(&key.as_str()) || OCR_TEXT_KEYS.contains(&key.as_str());
        fields.push((whole.start(), whole.end(), drop));
    }
    if !fields.iter().any(|f| f.2) {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    let mut i = 0;
    while i < fields.len() {
        let (start, value_start, drop) = fields[i];
        if start < at {
            // Inside a quoted value we already skipped.
            i += 1;
            continue;
        }
        if !drop {
            i += 1;
            continue;
        }
        let end = if line[value_start..].starts_with('"') {
            quoted_end(line, value_start)
        } else {
            fields
                .iter()
                .map(|f| f.0)
                .find(|&s| s > value_start)
                .unwrap_or(line.len())
        };
        out.push_str(&line[at..start]);
        at = end;
        i += 1;
    }
    out.push_str(&line[at..]);
    out
}

/// End (exclusive) of a Debug-quoted value that opens at `open`.
fn quoted_end(line: &str, open: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    line.len()
}

/// `log.txt` body: lines of `raw` (oldest first) inside `window`, colour
/// codes removed, name fields dropped, values scrubbed. Lines with no
/// timestamp are left out. Ends with a newline.
pub fn scrub_log(ctx: &ScrubContext, window: &LogWindow, raw: &str) -> String {
    let scrubber = Scrubber::new(ctx);
    let mut kept: Vec<String> = Vec::new();
    for line in raw.lines() {
        let line = strip_ansi(line);
        let Some(t) = line_timestamp(&line) else {
            continue;
        };
        if !window.contains(t) {
            continue;
        }
        let line = drop_name_fields(&line);
        kept.push(scrubber.scrub(line.trim_end()));
    }
    if kept.is_empty() {
        return format!("{NO_LINES_IN_WINDOW}\n");
    }
    let mut total: usize = kept.iter().map(|l| l.len() + 1).sum();
    let mut first = 0;
    let budget = LOG_MAX_BYTES - LOG_CUT_NOTE.len() - 1;
    if total > LOG_MAX_BYTES {
        while total > budget && first < kept.len() {
            total -= kept[first].len() + 1;
            first += 1;
        }
    }
    let mut out = String::with_capacity(total + LOG_CUT_NOTE.len() + 1);
    if first > 0 {
        out.push_str(LOG_CUT_NOTE);
        out.push('\n');
    }
    for line in &kept[first..] {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `daemon.log.1` then `daemon.log` from `data_dir`, oldest first. A missing
/// file is skipped. Bytes that are not UTF-8 are replaced.
pub fn read_daemon_logs(data_dir: &Path) -> String {
    read_daemon_logs_since(data_dir, None, None)
}

/// [`read_daemon_logs`] with the journal read from `since` (a game
/// report's window start), so a long session's lines are not cut by the
/// newest-lines cap alone. `until` (the window end) bounds the read so an
/// older game keeps its own lines after a long evening; with no end the
/// read runs to now. `None` for `since` reads the newest lines only.
pub fn read_daemon_logs_since(
    data_dir: &Path,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> String {
    let mut out = String::new();
    for name in ["daemon.log.1", "daemon.log"] {
        if let Ok(bytes) = std::fs::read(data_dir.join(name)) {
            out.push_str(&String::from_utf8_lossy(&bytes));
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    // The service logs to journald; daemon.log may be old or missing. The
    // window filter in [`scrub_log`] keeps only the game's own lines.
    if !cfg!(test) {
        out = merge_journal(out, &read_journal_lines(since, until));
    }
    out
}

/// `file` (the `daemon.log` lines) followed by the `journal` lines that
/// `file` does not already hold. While the daemon writes both, every line
/// shows up in each; it is kept once.
pub fn merge_journal(mut file: String, journal: &str) -> String {
    let seen: std::collections::HashSet<&str> = file.lines().collect();
    let extra: String = journal
        .lines()
        .filter(|l| !seen.contains(l))
        .flat_map(|l| [l, "\n"])
        .collect();
    file.push_str(&extra);
    file
}

/// The systemd user unit the tracker runs under.
pub const TRACKER_UNIT: &str = "scuffed-stat-tracker.service";

/// The tracker's own lines from the user journal, used when there is no
/// `daemon.log` (the service logs to journald). Only this unit is read
/// (`-u`), message text only (`-o cat`), so each line starts with the
/// tracker's own RFC 3339 timestamp and the window filter applies as for
/// the file. Read from `since` when given (the game's own start), and
/// always bounded to the newest [`JOURNAL_MAX_LINES`] lines as a fallback.
/// No journal, no lines.
pub fn read_journal_lines(since: Option<DateTime<Utc>>, until: Option<DateTime<Utc>>) -> String {
    let out = std::process::Command::new("journalctl")
        .args(journal_args(since, until, Utc::now()))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

/// Newest journal lines a report reads at most.
pub const JOURNAL_MAX_LINES: u32 = 20_000;

/// `journalctl` arguments for [`read_journal_lines`]. `since` is passed as a
/// Unix timestamp (`@<seconds>`), which every systemd reads the same way
/// whatever the local time zone. With `since`, the read is also bounded by
/// `--until` at `until` (the game window end), or at `now` when the game
/// has no end time, so the line cap keeps the game's own lines rather than
/// the newest ones of the evening.
pub fn journal_args(
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut args: Vec<String> = ["--user", "-u", TRACKER_UNIT, "-o", "cat", "--no-pager"]
        .into_iter()
        .map(String::from)
        .collect();
    if let Some(since) = since {
        args.push("--since".into());
        args.push(format!("@{}", since.timestamp()));
        args.push("--until".into());
        args.push(format!("@{}", until.unwrap_or(now).timestamp()));
    }
    args.push("-n".into());
    args.push(JOURNAL_MAX_LINES.to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    /// alpha.10: a game report reads the journal from the game's own start,
    /// and keeps the newest-lines cap as the fallback.
    #[test]
    fn the_journal_is_read_from_the_game_start_with_the_line_cap_kept() {
        let start = DateTime::parse_from_rfc3339("2026-10-15T08:20:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let args = journal_args(Some(start), None, start);
        let since = args.iter().position(|a| a == "--since").expect("--since");
        assert_eq!(args[since + 1], format!("@{}", start.timestamp()));
        assert!(args.windows(2).any(|w| w[0] == "-n" && w[1] == "20000"));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-u" && w[1] == TRACKER_UNIT)
        );
        let newest = journal_args(None, None, start);
        assert!(!newest.iter().any(|a| a == "--since"));
        assert!(!newest.iter().any(|a| a == "--until"));
        assert!(newest.windows(2).any(|w| w[0] == "-n" && w[1] == "20000"));
        let w = LogWindow::session(start, start + Duration::minutes(12));
        assert_eq!(w.start(), Some(start - Duration::seconds(120)));
    }

    /// QA note on #290: an older game's read stops at its own window end,
    /// so a long evening after it cannot push its lines out of the cap.
    #[test]
    fn the_journal_read_stops_at_the_game_window_end_or_now() {
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let w = LogWindow::session(t("2026-10-15T08:20:00Z"), t("2026-10-15T08:32:00Z"));
        let now = t("2026-10-15T23:00:00Z");
        let args = journal_args(w.start(), w.end(), now);
        let since = args.iter().position(|a| a == "--since").expect("--since");
        let until = args.iter().position(|a| a == "--until").expect("--until");
        assert_eq!(
            args[since + 1],
            format!("@{}", t("2026-10-15T08:18:00Z").timestamp())
        );
        assert_eq!(
            args[until + 1],
            format!("@{}", t("2026-10-15T08:34:00Z").timestamp())
        );
        assert!(since < until);
        assert_eq!(&args[args.len() - 2..], ["-n", "20000"]);
        assert_eq!(
            &args[..6],
            ["--user", "-u", TRACKER_UNIT, "-o", "cat", "--no-pager"]
        );

        let open = journal_args(w.start(), None, now);
        let until = open.iter().position(|a| a == "--until").expect("--until");
        assert_eq!(open[until + 1], format!("@{}", now.timestamp()));
    }

    #[test]
    fn a_line_in_both_daemon_log_and_the_journal_is_kept_once() {
        let file = "2026-10-14T21:00:00Z a\n2026-10-14T21:00:01Z b\n".to_string();
        let journal = "2026-10-14T21:00:01Z b\n2026-10-14T21:00:02Z c\n";
        assert_eq!(
            merge_journal(file, journal),
            "2026-10-14T21:00:00Z a\n2026-10-14T21:00:01Z b\n2026-10-14T21:00:02Z c\n"
        );
        assert_eq!(merge_journal(String::new(), "x\n"), "x\n");
    }

    fn ctx() -> ScrubContext {
        ScrubContext {
            sync_token: Some("sct_FAKEsyncTOKEN0123456789abcdef".into()),
            server_url: Some("https://crew.example.test/".into()),
            home_dir: Some(PathBuf::from("/home/qxuser")),
            account_name: Some("qxuser".into()),
            hostname: Some("qxhost-box".into()),
            player_name: Some("QXSelf#24680".into()),
            names: vec!["QXOther".into(), "QX0ther".into(), "Jo".into()],
        }
    }

    #[test]
    fn each_placeholder_row() {
        let c = ctx();
        let cases = [
            (
                "tok sct_FAKEsyncTOKEN0123456789abcdef end",
                "tok [sync-token] end",
            ),
            ("auth: BEARER abc.def", "auth: BEARER [sync-token]"),
            ("auth: bearer\tabc.def", "auth: bearer\t[sync-token]"),
            (
                "run ABCDEFGHIJKLMNOPQRSTUVWX_y-z.0 end",
                "run [sync-token] end",
            ),
            (
                "b64 QUJDREVGR0hJSktMTU5PUFFSU1RVVldY+/== end",
                "b64 [sync-token] end",
            ),
            (
                "url https://crew.example.test/api/stats?x=1 and http://CREW.example.test:8443/a",
                "url [server-url] and [server-url]",
            ),
            (
                "bare host crew.example.test here",
                "bare host [server-url] here",
            ),
            (
                "other https://other.example/x stays",
                "other https://other.example/x stays",
            ),
            ("p /home/qxuser/.local/share", "p [home]/.local/share"),
            ("p /var/home/someone/x", "p [home]/x"),
            ("p /Users/someone/x", "p [home]/x"),
            (
                "user qxuser and /mnt/qxuser/x",
                "user [user] and /mnt/[user]/x",
            ),
            ("not qxusers", "not qxusers"),
            ("on qxhost-box today", "on [host] today"),
            ("tag Ünïcødé#12345 end", "tag [battletag] end"),
            (
                "self QXSelf here and qxself",
                "self [player] here and [player]",
            ),
            (
                "names QXOther QX0ther Jo",
                "names [player] [player] [player]",
            ),
            ("Joined is not Jo", "Joined is not [player]"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    fn site_ctx(url: &str) -> ScrubContext {
        ScrubContext {
            server_url: Some(url.into()),
            ..ScrubContext::default()
        }
    }

    /// The daemon's off-site redirect line (sync client, `OffSiteRedirect`).
    fn redirect_line(target: &str) -> String {
        format!(
            "2026-10-24T10:00:00Z WARN sync: the site redirected sync to {target}; \
             if that is your site's new address, set the Website URL in Settings to it"
        )
    }

    const REDIRECT_SCRUBBED: &str = "2026-10-24T10:00:00Z WARN sync: the site redirected sync \
         to [server-url]; if that is your site's new address, set the Website URL in Settings to it";

    #[test]
    fn apex_site_scrubs_its_www_form() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            (
                "go https://www.crew.example.test/x now",
                "go [server-url] now",
            ),
            (
                "go http://WWW.Crew.Example.Test:8443 now",
                "go [server-url] now",
            ),
            ("bare www.crew.example.test here", "bare [server-url] here"),
            ("bare www.crew.example.test:8443.", "bare [server-url]."),
            ("sub https://api.crew.example.test/a", "sub [server-url]"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    #[test]
    fn www_site_scrubs_its_apex_form() {
        let c = site_ctx("https://www.crew.example.test/");
        let cases = [
            ("go https://crew.example.test/x now", "go [server-url] now"),
            (
                "go http://crew.example.test:8080, now",
                "go [server-url], now",
            ),
            ("bare crew.example.test here", "bare [server-url] here"),
            ("bare www.crew.example.test here", "bare [server-url] here"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    #[test]
    fn another_port_on_the_site_host_is_scrubbed() {
        let c = site_ctx("https://crew.example.test:8443");
        let cases = [
            ("a https://crew.example.test:9000/api b", "a [server-url] b"),
            ("a https://crew.example.test b", "a [server-url] b"),
            ("a crew.example.test:9000 b", "a [server-url] b"),
            ("a www.crew.example.test:1 b", "a [server-url] b"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    #[test]
    fn look_alike_hosts_are_not_scrubbed() {
        for site in ["https://crew.example.test", "https://www.crew.example.test"] {
            let c = site_ctx(site);
            for text in [
                "https://notcrew.example.test/x",
                "notcrew.example.test",
                "https://crew.example.test.ev.io/x",
                "crew.example.test.ev.io",
                "https://crew.example.test-ev.io",
                "crew.example.tester",
                "https://ev.io/crew",
            ] {
                assert_eq!(scrub_text(&c, text), text, "site {site}");
            }
        }
    }

    #[test]
    fn a_site_address_without_a_scheme_still_scrubs_its_host() {
        let c = site_ctx("crew.example.test:8443");
        assert_eq!(
            scrub_text(
                &c,
                "to https://www.crew.example.test/x and crew.example.test"
            ),
            "to [server-url] and [server-url]"
        );
        let c = site_ctx("not a url");
        assert_eq!(scrub_text(&c, "this is not a url"), "this is [server-url]");
    }

    /// Review repros on #494: each was scrubbed before the www/apex change
    /// and must stay scrubbed.
    #[test]
    fn a_rejected_longer_match_still_scrubs_the_apex() {
        let c = site_ctx("https://crew.x");
        let cases = [
            ("domain=.crew.x", "domain=.[server-url]"),
            ("see...crew.x", "see...[server-url]"),
            ("retrying...crew.x", "retrying...[server-url]"),
            ("ü.crew.x end", "[server-url] end"),
            ("bücher.crew.x end", "[server-url] end"),
            ("host crew.x:8443x", "host [server-url]:8443x"),
            ("host crew.x:8443abc", "host [server-url]:8443abc"),
            ("host crew.x:8443-", "host [server-url]:8443-"),
            ("host crew.x:8443.", "host [server-url]."),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
        let c = site_ctx("https://www.crew.example.test");
        assert_eq!(scrub_text(&c, "x=.crew.example.test"), "x=.[server-url]");
    }

    #[test]
    fn wrappers_around_a_site_url_survive() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            ("[https://www.crew.example.test]", "[[server-url]]"),
            ("{https://crew.example.test/a}", "{[server-url]}"),
            ("(https://crew.example.test/a).", "([server-url])."),
            ("see https://crew.example.test:8443!", "see [server-url]!"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
        let c = site_ctx("http://[2001:db8::1]:8443");
        assert_eq!(scrub_text(&c, "to http://[2001:db8::1]"), "to [server-url]");
    }

    #[test]
    fn an_idn_site_is_scrubbed_in_punycode_and_unicode() {
        for site in ["https://crëw.ex", "https://xn--crw-kma.ex"] {
            let c = site_ctx(site);
            let cases = [
                ("to https://www.xn--crw-kma.ex; x", "to [server-url]; x"),
                ("to https://xn--crw-kma.ex:8443 x", "to [server-url] x"),
                ("to https://www.crëw.ex/ x", "to [server-url] x"),
                (
                    "dns error for crëw.ex and XN--CRW-KMA.EX",
                    "dns error for [server-url] and [server-url]",
                ),
                ("other xn--crw-dma.ex stays", "other xn--crw-dma.ex stays"),
            ];
            for (input, want) in cases {
                assert_eq!(scrub_text(&c, input), want, "site {site} input: {input}");
            }
            let got = scrub_text(&c, &redirect_line("https://www.xn--crw-kma.ex"));
            assert_eq!(got, REDIRECT_SCRUBBED, "site {site}");
        }
    }

    #[test]
    fn a_site_configured_with_a_trailing_dot_still_scrubs() {
        let c = site_ctx("https://crew.example.test./");
        let cases = [
            ("a https://crew.example.test/a b", "a [server-url] b"),
            ("a crew.example.test b", "a [server-url] b"),
            ("a crew.example.test. b", "a [server-url]. b"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
        let got = scrub_text(&c, &redirect_line("https://www.crew.example.test"));
        assert_eq!(got, REDIRECT_SCRUBBED);
    }

    #[test]
    fn a_site_address_with_no_host_never_scrubs_scheme_words() {
        let text = "https://crew.example.test/a https http://x.io/b";
        for site in [
            "https://",
            "https:///x",
            "http://:8443",
            "https://.",
            "https://@/",
        ] {
            assert_eq!(scrub_text(&site_ctx(site), text), text, "site {site}");
        }
    }

    #[test]
    fn percent_encoded_site_urls_are_scrubbed() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            (
                "u=https%3A%2F%2Fcrew.example.test%2Fx",
                "u=[server-url]%2Fx",
            ),
            ("u=http%3a%2f%2fwww.crew.example.test&y", "u=[server-url]&y"),
            ("u=%40crew.example.test", "u=%40[server-url]"),
            ("u=crew%2Eexample%2etest%2F", "u=[server-url]%2F"),
            ("u=%41crew.example.test", "u=%41crew.example.test"),
            ("u=crew.example.test%2Eev.io", "u=crew.example.test%2Eev.io"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    /// Review repros on #494: a `%` before a non-ASCII label must not
    /// slice inside a character.
    #[test]
    fn a_percent_before_a_non_ascii_label_does_not_panic() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            ("%aé.crew.example.test", "%aé.[server-url]"),
            ("x%aü.crew.example.test", "x%aü.[server-url]"),
            ("%aéé.crew.example.test", "%aéé.[server-url]"),
            ("é%aé.crew.example.test", "é%aé.[server-url]"),
            ("%1日.crew.example.test", "%1日.[server-url]"),
            ("%2Fé.crew.example.test", "%2F[server-url]"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    /// Each label walk stops at the previous match, so many mentions in a
    /// row stay linear (this took about 113 s before the floor).
    #[test]
    fn many_mentions_with_non_ascii_labels_stay_fast() {
        let c = site_ctx("https://crew.example.test");
        let text = "ü.crew.example.test".repeat(20_000);
        let started = std::time::Instant::now();
        let out = scrub_text(&c, &text);
        let took = started.elapsed();
        assert!(!out.contains("crew.example.test"));
        assert!(took < std::time::Duration::from_secs(10), "took {took:?}");
        let long_label = format!("{}.crew.example.test", "é".repeat(100_000));
        let started = std::time::Instant::now();
        let out = scrub_text(&c, &long_label);
        assert!(out.ends_with("[server-url]"), "{}", &out[out.len() - 40..]);
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_secs(10), "took {took:?}");
    }

    /// Fuzz-style: random mixes of `%`, hex digits, non-ASCII letters,
    /// dots, ports and the site host never panic.
    #[test]
    fn random_text_around_the_site_host_never_panics() {
        let pieces = [
            "%",
            "%2",
            "%2F",
            "%2E",
            "%e",
            "a",
            "é",
            "日",
            "ü",
            ".",
            ":",
            "8443",
            "-",
            "_",
            " ",
            "/",
            "@",
            "[",
            "]",
            "crew.example.test",
            "crew%2Eexample",
            "www.",
            "xn--crw-kma.ex",
            "crëw.ex",
            "https://",
            "https%3A%2F%2F",
            "%25",
            "%252E",
            "%252F",
            "%253A",
            "%63",
            "www%2E",
            "\u{200B}",
            "\u{200C}",
            "\u{200D}",
            "\u{FEFF}",
            "\u{AD}",
            "。",
            "．",
            "｡",
        ];
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for (site, host) in [
            ("https://crew.example.test", "crew.example.test"),
            ("https://crëw.ex", "crëw.ex"),
            ("https://crëw。ex", "crëw.ex"),
        ] {
            let scrubber = Scrubber::new(&site_ctx(site));
            let clean = format!(" {host} ");
            for _ in 0..3_000 {
                let len = (next() % 24) as usize;
                let text: String = (0..len)
                    .map(|_| pieces[(next() % pieces.len() as u64) as usize])
                    .collect();
                let _ = scrubber.scrub(&text);
                // A clean mention among the noise is always hidden.
                let out = scrubber.scrub(&format!("{text}{clean}{text}"));
                assert!(!out.contains(&clean), "site {site} text {text:?}");
            }
        }
    }

    /// #494 follow-up: a URL encoded twice or more (a link inside a link
    /// inside a query) is read through each `%25` layer, up to 3 deep.
    #[test]
    fn double_percent_encoded_site_urls_are_scrubbed() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            ("u=%252F%252Fcrew.example.test", "u=%252F%252F[server-url]"),
            (
                "u=https%253A%252F%252Fcrew.example.test%252Fx",
                "u=[server-url]%252Fx",
            ),
            ("u=crew%252Eexample%252etest&y", "u=[server-url]&y"),
            ("u=crew%25252Eexample.test", "u=[server-url]"),
            // Four layers is past the bound and stays.
            ("u=crew%2525252Eexample.test", "u=crew%2525252Eexample.test"),
            // An encoded `A` in front, or a label behind, is a look-alike.
            ("u=%2541crew.example.test", "u=%2541crew.example.test"),
            (
                "u=crew.example.test%252Eev.io",
                "u=crew.example.test%252Eev.io",
            ),
            // Broken escapes are plain text.
            (
                "%25 %25% %252 %25zz crew.example.test",
                "%25 %25% %252 %25zz [server-url]",
            ),
            ("crew.example.test%25", "[server-url]%25"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    /// #494 follow-up: an escape that encodes a host letter counts as that
    /// letter, with either case of hex digit.
    #[test]
    fn percent_encoded_letters_in_the_host_are_scrubbed() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            ("u=%63rew.example.test", "u=[server-url]"),
            ("u=%43REW.%65xample.tes%74/x", "u=[server-url]/x"),
            ("u=cr%65w%2eexample%2Etest", "u=[server-url]"),
            ("u=%2563rew.example.test", "u=[server-url]"),
            ("u=%E9crew.example.test", "u=%E9[server-url]"),
            ("u=%6Eotcrew.example.test", "u=%6Eotcrew.example.test"),
            ("u=crew.example.tes%74er", "u=crew.example.tes%74er"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    /// #494 follow-up: zero-width characters, the byte-order mark and the
    /// soft hyphen inside the host are skipped, and hidden with it.
    #[test]
    fn zero_width_characters_inside_the_host_are_scrubbed() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            ("a crew\u{200B}.example.test b", "a [server-url] b"),
            ("a c\u{200C}rew.ex\u{200D}ample.test b", "a [server-url] b"),
            ("a crew.\u{FEFF}example.test b", "a [server-url] b"),
            ("a cr\u{AD}ew.example.test b", "a [server-url] b"),
            ("a www\u{200B}.crew.example.test:8443 b", "a [server-url] b"),
            (
                "a https://crew\u{200B}.example.test/p b",
                "a [server-url] b",
            ),
            // Next to the host it is a break, as before.
            (
                "a \u{200B}crew.example.test\u{200B} b",
                "a \u{200B}[server-url]\u{200B} b",
            ),
            // Look-alikes stay.
            ("crew\u{200B}.example.tester", "crew\u{200B}.example.tester"),
            (
                "notcrew\u{200B}.example.test",
                "notcrew\u{200B}.example.test",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input:?}");
        }
    }

    /// #494 follow-up: an IDNA full stop (`。`, `．`, `｡`) in the Website
    /// URL is mapped to a dot first, so the site is found in both forms.
    #[test]
    fn a_site_typed_with_an_ideographic_full_stop_is_normalised() {
        for site in [
            "https://crëw。ex",
            "https://crëw．ex/",
            "crëw｡ex",
            "https://xn--crw-kma。ex",
        ] {
            let c = site_ctx(site);
            let got = scrub_text(&c, &redirect_line("https://www.xn--crw-kma.ex"));
            assert_eq!(got, REDIRECT_SCRUBBED, "site {site}");
            let cases = [
                (
                    "dns crëw.ex and xn--crw-kma.ex:8443",
                    "dns [server-url] and [server-url]",
                ),
                ("other xn--crw-dma.ex stays", "other xn--crw-dma.ex stays"),
                ("other crëw。ex", "other [server-url]"),
            ];
            for (input, want) in cases {
                assert_eq!(scrub_text(&c, input), want, "site {site} input: {input}");
            }
        }
    }

    /// #494 follow-up: in log text an IDNA full stop counts as a dot
    /// between labels. After the host it ends a sentence.
    #[test]
    fn idna_full_stops_in_the_log_count_as_dots() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            ("a crew。example。test b", "a [server-url] b"),
            ("a www．crew.example.test b", "a [server-url] b"),
            ("a crew｡example.test:8443 b", "a [server-url] b"),
            ("a https://www。crew.example.test/x b", "a [server-url] b"),
            ("see crew.example.test。次", "see [server-url]。次"),
            ("see crew.example.test。Then", "see [server-url]。Then"),
            ("notcrew。example.test", "notcrew。example.test"),
            ("crew。example。tester", "crew。example。tester"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    /// #494 follow-up: an encoded `www%2E` in front and an encoded
    /// `%3A<port>` behind are hidden with the host.
    #[test]
    fn an_encoded_www_and_port_go_with_the_host() {
        let c = site_ctx("https://crew.example.test");
        let cases = [
            (
                "x%2F%2Fwww%2Ecrew%2Eexample%2Etest%3A8443%2Fp",
                "x%2F%2F[server-url]%2Fp",
            ),
            ("host=crew.example.test%3A8443", "host=[server-url]"),
            (
                "u=https%3A%2F%2Fwww%2Ecrew.example.test%3A8443%2Fx",
                "u=[server-url]%2Fx",
            ),
            (
                "u=www%252Ecrew.example.test%253A8443%252F",
                "u=[server-url]%252F",
            ),
            ("crew.example.test%3A8443x", "[server-url]%3A8443x"),
        ];
        for (input, want) in cases {
            assert_eq!(scrub_text(&c, input), want, "input: {input}");
        }
    }

    /// #494 follow-up: matches merge in one pass after one sort. 100k
    /// mentions took about 18 s before.
    #[test]
    fn many_site_mentions_merge_fast() {
        let c = site_ctx("https://crew.example.test");
        let text = "see crew.example.test ".repeat(100_000);
        assert!(text.len() > 2_000_000);
        let started = std::time::Instant::now();
        let out = scrub_text(&c, &text);
        let took = started.elapsed();
        assert_eq!(out.matches(PH_SERVER_URL).count(), 100_000);
        assert!(!out.contains("crew.example.test"));
        assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
    }

    /// A match that starts inside the port just taken is skipped. Before,
    /// its label walk sliced the text backwards and panicked.
    #[test]
    fn a_match_inside_a_taken_port_does_not_panic() {
        let c = site_ctx("http://4");
        assert_eq!(scrub_text(&c, "4:4"), "[server-url]");
        assert_eq!(
            scrub_text(&c, "a 4:4 b 4:44"),
            "a [server-url] b [server-url]"
        );
    }

    #[test]
    fn overlapping_spans_merge_and_keep_the_longest_placeholder() {
        let span = |start, end, placeholder| Span {
            start,
            end,
            placeholder,
        };
        let text = "abcdefghij";
        // Overlap: one span, the longer match's placeholder.
        let got = apply_spans(text, vec![span(0, 4, "[a]"), span(2, 8, "[b]")]);
        assert_eq!(got, "[b]ij");
        // Touching spans stay apart.
        let got = apply_spans(text, vec![span(2, 4, "[b]"), span(0, 2, "[a]")]);
        assert_eq!(got, "[a][b]efghij");
        // A chain merges whole; a tie goes to the first match.
        let got = apply_spans(
            text,
            vec![span(4, 6, "[c]"), span(0, 3, "[a]"), span(2, 5, "[b]")],
        );
        assert_eq!(got, "[a]ghij");
        // Empty spans and a span inside another are dropped.
        let got = apply_spans(
            text,
            vec![span(1, 1, "[e]"), span(1, 9, "[x]"), span(3, 4, "[y]")],
        );
        assert_eq!(got, "a[x]j");
    }

    #[test]
    fn redirect_log_line_is_scrubbed_both_ways() {
        let cases = [
            ("https://crew.example.test", "https://www.crew.example.test"),
            ("https://www.crew.example.test", "https://crew.example.test"),
            (
                "https://crew.example.test",
                "https://crew.example.test:8443",
            ),
        ];
        for (site, target) in cases {
            let got = scrub_text(&site_ctx(site), &redirect_line(target));
            assert_eq!(got, REDIRECT_SCRUBBED, "site {site} to {target}");
        }
    }

    #[test]
    fn longer_match_wins_and_overlaps_merge() {
        let c = ctx();
        // Home path contains the account name: the longer home match wins.
        assert_eq!(scrub_text(&c, "/home/qxuser"), "[home]");
        // BattleTag of the configured player: whole tag goes.
        assert_eq!(scrub_text(&c, "QXSelf#24680 joined"), "[battletag] joined");
        // Placeholders are not scrubbed again.
        let c2 = ScrubContext {
            names: vec!["player".into(), "home".into()],
            ..ctx()
        };
        assert_eq!(scrub_text(&c2, "player /home/x"), "[player] [home]");
    }

    #[test]
    fn name_fields_are_dropped() {
        let line = "2026-10-09T12:00:00.000001Z  INFO stat_tracker: player_name fetched from server player_name=QXSelf#24680 other=1";
        assert_eq!(
            drop_name_fields(line),
            "2026-10-09T12:00:00.000001Z  INFO stat_tracker: player_name fetched from server other=1"
        );
        let quoted = r#"t INFO x: row read name="QX Name \"q\" x=1" row=3"#;
        assert_eq!(drop_name_fields(quoted), "t INFO x: row read row=3");
        let spaced = "t DEBUG x: player name found via substring in row row=2 text=qx other name";
        assert_eq!(
            drop_name_fields(spaced),
            "t DEBUG x: player name found via substring in row row=2"
        );
        // Raw OCR text is dropped on every line (it can hold names), so
        // only the message stays.
        let plain = "t INFO x: outcome read text=VICTORY";
        assert_eq!(drop_name_fields(plain), "t INFO x: outcome read");
        let kept = "t INFO x: outcome read outcome=Victory";
        assert_eq!(drop_name_fields(kept), kept);
    }

    #[test]
    fn window_rules() {
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let w = LogWindow::session(t("2026-10-09T12:00:00Z"), t("2026-10-09T12:10:00Z"));
        assert!(w.contains(t("2026-10-09T11:58:00Z")));
        assert!(!w.contains(t("2026-10-09T11:57:59Z")));
        assert!(w.contains(t("2026-10-09T12:12:00Z")));
        assert!(!w.contains(t("2026-10-09T12:12:01Z")));

        let w = LogWindow::log_only(t("2026-10-09T12:00:00Z"));
        assert!(w.contains(t("2026-10-09T11:30:00Z")));
        assert!(!w.contains(t("2026-10-09T11:29:59Z")));
        assert!(!w.contains(t("2026-10-09T12:00:01Z")));

        // Crash: last_seen - 30 min ..= last_seen + 2 min, plus the build window.
        let w = LogWindow::crash(t("2026-10-09T12:00:00Z"), t("2026-10-08T09:00:00Z"));
        assert!(w.contains(t("2026-10-08T08:30:00Z")));
        assert!(!w.contains(t("2026-10-08T08:29:59Z")));
        assert!(w.contains(t("2026-10-08T09:02:00Z")));
        assert!(!w.contains(t("2026-10-08T09:02:01Z")));
        assert!(w.contains(t("2026-10-09T11:30:00Z")));
        assert!(!w.contains(t("2026-10-09T11:29:59Z")));
        // Overlapping windows: a line in either is kept.
        let w = LogWindow::crash(t("2026-10-09T12:00:00Z"), t("2026-10-09T11:50:00Z"));
        assert!(w.contains(t("2026-10-09T11:20:00Z")));
        assert!(w.contains(t("2026-10-09T11:59:00Z")));
        assert!(!w.contains(t("2026-10-09T11:19:59Z")));

        assert_eq!(
            LogWindow::for_log_only(
                t("2026-10-09T12:00:00Z"),
                false,
                Some(t("2026-10-08T09:00:00Z"))
            ),
            LogWindow::log_only(t("2026-10-09T12:00:00Z"))
        );
        assert_eq!(
            LogWindow::for_log_only(t("2026-10-09T12:00:00Z"), true, None),
            LogWindow::log_only(t("2026-10-09T12:00:00Z"))
        );
    }

    #[test]
    fn empty_window_and_untimed_lines() {
        let t = DateTime::parse_from_rfc3339("2026-10-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let w = LogWindow::log_only(t);
        let raw = "no timestamp here\n2020-01-01T00:00:00Z INFO x: old\n";
        assert_eq!(scrub_log(&ctx(), &w, raw), "[no log lines in window]\n");
    }

    #[test]
    fn ansi_is_stripped_before_the_timestamp_parse() {
        let raw = "\x1b[2m2026-10-09T11:59:00.123456Z\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mx\x1b[0m\x1b[2m:\x1b[0m hi";
        let t = DateTime::parse_from_rfc3339("2026-10-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let out = scrub_log(&ctx(), &LogWindow::log_only(t), raw);
        assert_eq!(out, "2026-10-09T11:59:00.123456Z  INFO x: hi\n");
    }

    #[test]
    fn oversized_log_keeps_the_newest_lines() {
        let t = DateTime::parse_from_rfc3339("2026-10-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let line = format!("2026-10-09T11:59:00Z INFO x: {}", "a b ".repeat(100));
        let mut raw = String::new();
        for i in 0..12_000 {
            raw.push_str(&format!("{line} n{i}\n"));
        }
        let out = scrub_log(&ScrubContext::default(), &LogWindow::log_only(t), &raw);
        assert!(out.len() <= LOG_MAX_BYTES);
        assert!(out.starts_with(LOG_CUT_NOTE));
        assert!(out.trim_end().ends_with("n11999"));
    }
}

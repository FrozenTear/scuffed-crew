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
const URL_TRAILING_PUNCT: &[char] = &['.', ',', ';', ':', '!', '?', ')'];
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
    /// Lowercase host without a leading `www.`.
    apex: String,
    /// The apex with any labels in front (`www.`, `api.`) and any port.
    bare_re: Regex,
}

impl SiteHost {
    fn new(host: &str) -> Option<Self> {
        let host = host.to_ascii_lowercase();
        // `www.example` keeps its `www.`: a single label is not a site.
        let apex = match host.strip_prefix("www.") {
            Some(rest) if rest.contains('.') => rest.to_string(),
            _ => host,
        };
        let pattern = format!(
            r"(?i)(?:[a-z0-9_-]+\.)*{}(?::[0-9]+)?",
            regex::escape(&apex)
        );
        let bare_re = Regex::new(&pattern).ok()?;
        Some(Self { apex, bare_re })
    }

    /// `host` is the site: the apex itself or a name under it.
    fn is_site(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        host == self.apex
            || host
                .strip_suffix(self.apex.as_str())
                .is_some_and(|front| front.ends_with('.'))
    }

    /// Bare mentions of the site host. A match must not sit inside a longer
    /// host name: `notcrew.example` and `crew.example.evil.com` are other
    /// sites and stay.
    fn push_bare(&self, text: &str, out: &mut Vec<Span>) {
        for m in self.bare_re.find_iter(text) {
            let before = text[..m.start()].chars().next_back();
            if before.is_some_and(|c| is_host_char(c) || c == '.') {
                continue;
            }
            let mut after = text[m.end()..].chars();
            match after.next() {
                Some(c) if is_host_char(c) => continue,
                Some('.') if after.next().is_some_and(is_host_char) => continue,
                _ => {}
            }
            out.push(Span {
                start: m.start(),
                end: m.end(),
                placeholder: PH_SERVER_URL,
            });
        }
    }
}

fn is_host_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
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
        // with no host that can be read.
        let server_url = non_empty(ctx.server_url.as_deref());
        let site = server_url
            .and_then(|u| url_host(u).or_else(|| bare_host(u)))
            .and_then(|h| SiteHost::new(&h));
        if let (Some(url), None) = (server_url, &site) {
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
                let url = m.as_str().trim_end_matches(URL_TRAILING_PUNCT);
                if url_host(url).is_some_and(|h| site.is_site(&h)) {
                    spans.push(Span {
                        start: m.start(),
                        end: m.start() + url.len(),
                        placeholder: PH_SERVER_URL,
                    });
                }
            }
            site.push_bare(text, &mut spans);
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

/// Longest match first. A match overlapping an accepted one is merged into
/// it and keeps the longer match's placeholder.
fn apply_spans(text: &str, mut spans: Vec<Span>) -> String {
    spans.sort_by(|a, b| {
        (b.end - b.start)
            .cmp(&(a.end - a.start))
            .then(a.start.cmp(&b.start))
    });
    let mut accepted: Vec<Span> = Vec::new();
    for s in spans {
        if s.start == s.end {
            continue;
        }
        let mut merged = s;
        let mut placeholder = None;
        accepted.retain(|a| {
            let overlaps = a.start < merged.end && merged.start < a.end;
            if overlaps {
                placeholder.get_or_insert(a.placeholder);
                merged.start = merged.start.min(a.start);
                merged.end = merged.end.max(a.end);
            }
            !overlaps
        });
        if let Some(ph) = placeholder {
            merged.placeholder = ph;
        }
        accepted.push(merged);
    }
    accepted.sort_by_key(|s| s.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for s in accepted {
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
    addr.chars()
        .all(|c| is_host_char(c) || matches!(c, '.' | ':' | '/'))
        .then(|| url_host(&format!("https://{addr}")))
        .flatten()
}

/// Lowercase host of `url`, without port or user info.
fn url_host(url: &str) -> Option<String> {
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

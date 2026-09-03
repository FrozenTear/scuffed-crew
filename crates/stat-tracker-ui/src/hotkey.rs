//! Companion overlay show/hide shortcut.
//!
//! The overlay surface is `KeyboardInteractivity::None` (click-through), so it
//! cannot see keys. This listens on `/dev/input` the same way the daemon
//! listens for Tab — evdev, no X11 grab, works on niri / Wayland.
//!
//! niri does not implement the xdg-desktop-portal GlobalShortcuts backend, so
//! that portal is not used. Optional compositor bind: touch
//! `<data_dir>/companion_toggle` (see README).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use evdev::{Device, EventSummary, KeyCode};
use iced::Subscription;
use iced::futures::SinkExt;
use tokio::sync::mpsc;

use crate::app::Message;

/// Default bind. Super is the Windows / Meta key.
pub const DEFAULT_BIND: &str = "Super+Shift+C";

const SCAN_EVERY: Duration = Duration::from_secs(3);
const IDLE_POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayHotkey {
    pub enabled: bool,
    pub bind: String,
}

impl Default for OverlayHotkey {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: DEFAULT_BIND.to_string(),
        }
    }
}

impl OverlayHotkey {
    pub fn normalized(enabled: bool, bind: &str) -> Self {
        let trimmed = bind.trim();
        Self {
            enabled,
            bind: if trimmed.is_empty() {
                DEFAULT_BIND.to_string()
            } else {
                trimmed.to_string()
            },
        }
    }

    /// One-line footer when the overlay is on screen.
    pub fn footer_hint(&self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let display = parse_bind(&self.bind)
            .map(|b| b.to_string())
            .unwrap_or_else(|_| DEFAULT_BIND.to_string());
        Some(format!("Hide or show with {display}"))
    }

    /// `Ok` with a display form, or `Err` when enabled and the bind is invalid.
    pub fn validate_for_save(&self) -> Result<Self, String> {
        let next = Self::normalized(self.enabled, &self.bind);
        if next.enabled {
            parse_bind(&next.bind)?;
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keybind {
    pub super_key: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: KeyCode,
}

impl fmt::Display for Keybind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.super_key {
            parts.push("Super");
        }
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(key_label(self.key));
        f.write_str(&parts.join("+"))
    }
}

/// Parse `Super+Shift+C`, `Meta+Shift+c`, `Ctrl+Alt+O`. Join with `+`.
pub fn parse_bind(raw: &str) -> Result<Keybind, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return parse_bind(DEFAULT_BIND);
    }
    let tokens: Vec<&str> = trimmed
        .split('+')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err(invalid_bind_copy());
    }
    let (key_tok, mod_toks) = tokens.split_last().expect("tokens non-empty");
    let Some(key) = key_from_name(key_tok) else {
        return Err(invalid_bind_copy());
    };
    if is_modifier_name(key_tok) {
        return Err(invalid_bind_copy());
    }
    let mut bind = Keybind {
        super_key: false,
        ctrl: false,
        alt: false,
        shift: false,
        key,
    };
    for tok in mod_toks {
        match norm_token(tok).as_str() {
            "SUPER" | "META" | "WIN" | "WINDOWS" | "MOD4" => {
                if bind.super_key {
                    return Err(invalid_bind_copy());
                }
                bind.super_key = true;
            }
            "CTRL" | "CONTROL" => {
                if bind.ctrl {
                    return Err(invalid_bind_copy());
                }
                bind.ctrl = true;
            }
            "ALT" | "MOD1" => {
                if bind.alt {
                    return Err(invalid_bind_copy());
                }
                bind.alt = true;
            }
            "SHIFT" => {
                if bind.shift {
                    return Err(invalid_bind_copy());
                }
                bind.shift = true;
            }
            _ => return Err(invalid_bind_copy()),
        }
    }
    if !bind.super_key && !bind.ctrl && !bind.alt && !bind.shift {
        return Err(invalid_bind_copy());
    }
    Ok(bind)
}

fn invalid_bind_copy() -> String {
    "Use a shortcut like Super+Shift+C — Super is the Windows key, join keys with +.".into()
}

fn norm_token(tok: &str) -> String {
    tok.trim().to_ascii_uppercase()
}

fn is_modifier_name(tok: &str) -> bool {
    matches!(
        norm_token(tok).as_str(),
        "SUPER"
            | "META"
            | "WIN"
            | "WINDOWS"
            | "MOD4"
            | "CTRL"
            | "CONTROL"
            | "ALT"
            | "MOD1"
            | "SHIFT"
    )
}

fn key_from_name(tok: &str) -> Option<KeyCode> {
    let u = norm_token(tok);
    if u.len() == 1 {
        let c = u.chars().next()?;
        return match c {
            'A'..='Z' => Some(letter_key(c)),
            '0'..='9' => Some(digit_key(c)),
            _ => None,
        };
    }
    if let Some(rest) = u.strip_prefix('F')
        && let Ok(n) = rest.parse::<u8>()
    {
        return f_key(n);
    }
    match u.as_str() {
        "TAB" => Some(KeyCode::KEY_TAB),
        "SPACE" | "SPC" => Some(KeyCode::KEY_SPACE),
        "ESC" | "ESCAPE" => Some(KeyCode::KEY_ESC),
        "ENTER" | "RETURN" => Some(KeyCode::KEY_ENTER),
        "BACKSPACE" => Some(KeyCode::KEY_BACKSPACE),
        _ => None,
    }
}

fn letter_key(c: char) -> KeyCode {
    match c {
        'A' => KeyCode::KEY_A,
        'B' => KeyCode::KEY_B,
        'C' => KeyCode::KEY_C,
        'D' => KeyCode::KEY_D,
        'E' => KeyCode::KEY_E,
        'F' => KeyCode::KEY_F,
        'G' => KeyCode::KEY_G,
        'H' => KeyCode::KEY_H,
        'I' => KeyCode::KEY_I,
        'J' => KeyCode::KEY_J,
        'K' => KeyCode::KEY_K,
        'L' => KeyCode::KEY_L,
        'M' => KeyCode::KEY_M,
        'N' => KeyCode::KEY_N,
        'O' => KeyCode::KEY_O,
        'P' => KeyCode::KEY_P,
        'Q' => KeyCode::KEY_Q,
        'R' => KeyCode::KEY_R,
        'S' => KeyCode::KEY_S,
        'T' => KeyCode::KEY_T,
        'U' => KeyCode::KEY_U,
        'V' => KeyCode::KEY_V,
        'W' => KeyCode::KEY_W,
        'X' => KeyCode::KEY_X,
        'Y' => KeyCode::KEY_Y,
        'Z' => KeyCode::KEY_Z,
        _ => KeyCode::KEY_C,
    }
}

fn digit_key(c: char) -> KeyCode {
    match c {
        '1' => KeyCode::KEY_1,
        '2' => KeyCode::KEY_2,
        '3' => KeyCode::KEY_3,
        '4' => KeyCode::KEY_4,
        '5' => KeyCode::KEY_5,
        '6' => KeyCode::KEY_6,
        '7' => KeyCode::KEY_7,
        '8' => KeyCode::KEY_8,
        '9' => KeyCode::KEY_9,
        _ => KeyCode::KEY_0,
    }
}

fn f_key(n: u8) -> Option<KeyCode> {
    Some(match n {
        1 => KeyCode::KEY_F1,
        2 => KeyCode::KEY_F2,
        3 => KeyCode::KEY_F3,
        4 => KeyCode::KEY_F4,
        5 => KeyCode::KEY_F5,
        6 => KeyCode::KEY_F6,
        7 => KeyCode::KEY_F7,
        8 => KeyCode::KEY_F8,
        9 => KeyCode::KEY_F9,
        10 => KeyCode::KEY_F10,
        11 => KeyCode::KEY_F11,
        12 => KeyCode::KEY_F12,
        _ => return None,
    })
}

fn key_label(key: KeyCode) -> &'static str {
    match key {
        KeyCode::KEY_A => "A",
        KeyCode::KEY_B => "B",
        KeyCode::KEY_C => "C",
        KeyCode::KEY_D => "D",
        KeyCode::KEY_E => "E",
        KeyCode::KEY_F => "F",
        KeyCode::KEY_G => "G",
        KeyCode::KEY_H => "H",
        KeyCode::KEY_I => "I",
        KeyCode::KEY_J => "J",
        KeyCode::KEY_K => "K",
        KeyCode::KEY_L => "L",
        KeyCode::KEY_M => "M",
        KeyCode::KEY_N => "N",
        KeyCode::KEY_O => "O",
        KeyCode::KEY_P => "P",
        KeyCode::KEY_Q => "Q",
        KeyCode::KEY_R => "R",
        KeyCode::KEY_S => "S",
        KeyCode::KEY_T => "T",
        KeyCode::KEY_U => "U",
        KeyCode::KEY_V => "V",
        KeyCode::KEY_W => "W",
        KeyCode::KEY_X => "X",
        KeyCode::KEY_Y => "Y",
        KeyCode::KEY_Z => "Z",
        KeyCode::KEY_0 => "0",
        KeyCode::KEY_1 => "1",
        KeyCode::KEY_2 => "2",
        KeyCode::KEY_3 => "3",
        KeyCode::KEY_4 => "4",
        KeyCode::KEY_5 => "5",
        KeyCode::KEY_6 => "6",
        KeyCode::KEY_7 => "7",
        KeyCode::KEY_8 => "8",
        KeyCode::KEY_9 => "9",
        KeyCode::KEY_F1 => "F1",
        KeyCode::KEY_F2 => "F2",
        KeyCode::KEY_F3 => "F3",
        KeyCode::KEY_F4 => "F4",
        KeyCode::KEY_F5 => "F5",
        KeyCode::KEY_F6 => "F6",
        KeyCode::KEY_F7 => "F7",
        KeyCode::KEY_F8 => "F8",
        KeyCode::KEY_F9 => "F9",
        KeyCode::KEY_F10 => "F10",
        KeyCode::KEY_F11 => "F11",
        KeyCode::KEY_F12 => "F12",
        KeyCode::KEY_TAB => "Tab",
        KeyCode::KEY_SPACE => "Space",
        KeyCode::KEY_ESC => "Esc",
        KeyCode::KEY_ENTER => "Enter",
        KeyCode::KEY_BACKSPACE => "Backspace",
        _ => "Key",
    }
}

#[derive(Debug, Clone)]
pub struct ComboState {
    bind: Keybind,
    super_down: bool,
    ctrl_down: bool,
    alt_down: bool,
    shift_down: bool,
    fired: bool,
}

impl ComboState {
    pub fn new(bind: Keybind) -> Self {
        Self {
            bind,
            super_down: false,
            ctrl_down: false,
            alt_down: false,
            shift_down: false,
            fired: false,
        }
    }

    /// Feed one evdev key event (`value`: 0 release, 1 press, 2 repeat).
    /// Returns true on the first matching press (not repeat).
    pub fn push(&mut self, code: KeyCode, value: i32) -> bool {
        let down = value != 0;
        match code {
            KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => {
                self.super_down = down;
                if !down {
                    self.fired = false;
                }
                false
            }
            KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => {
                self.ctrl_down = down;
                if !down {
                    self.fired = false;
                }
                false
            }
            KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => {
                self.alt_down = down;
                if !down {
                    self.fired = false;
                }
                false
            }
            KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => {
                self.shift_down = down;
                if !down {
                    self.fired = false;
                }
                false
            }
            other if other == self.bind.key => {
                if value == 0 {
                    self.fired = false;
                    return false;
                }
                if value == 1 && !self.fired && self.mods_match() {
                    self.fired = true;
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn mods_match(&self) -> bool {
        self.super_down == self.bind.super_key
            && self.ctrl_down == self.bind.ctrl
            && self.alt_down == self.bind.alt
            && self.shift_down == self.bind.shift
    }
}

pub fn toggle_path(data_dir: &Path) -> PathBuf {
    data_dir.join("companion_toggle")
}

/// True when `<data_dir>/companion_toggle` mtime advanced since `last`.
/// An existing file at first sight does not fire (startup).
pub fn consume_toggle(data_dir: &Path, last: &mut Option<SystemTime>) -> bool {
    let mtime = std::fs::metadata(toggle_path(data_dir))
        .and_then(|m| m.modified())
        .ok();
    match (mtime, *last) {
        (Some(now), Some(prev)) if now > prev => {
            *last = Some(now);
            true
        }
        (Some(now), None) => {
            *last = Some(now);
            false
        }
        (None, _) => {
            *last = None;
            false
        }
        _ => false,
    }
}

pub fn subscription(bind: Keybind) -> Subscription<Message> {
    Subscription::run_with_id(
        format!("companion-hotkey:{bind}"),
        iced::stream::channel(4, move |mut output| async move {
            let (tx, mut rx) = mpsc::unbounded_channel();
            if let Err(e) = std::thread::Builder::new()
                .name("companion-hotkey".into())
                .spawn(move || evdev_loop(bind, tx))
            {
                tracing::warn!(error = %e, "companion shortcut thread failed to start");
                return;
            }
            while rx.recv().await.is_some() {
                if output.send(Message::ToggleOverlay).await.is_err() {
                    break;
                }
            }
        }),
    )
}

fn is_keyboard(device: &Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_TAB)
            && keys.contains(KeyCode::KEY_A)
            && keys.contains(KeyCode::KEY_ENTER)
    })
}

fn evdev_loop(bind: Keybind, tx: mpsc::UnboundedSender<()>) {
    let mut devices: HashMap<PathBuf, Device> = HashMap::new();
    let mut combo = ComboState::new(bind);
    let mut last_scan = Instant::now()
        .checked_sub(SCAN_EVERY)
        .unwrap_or_else(Instant::now);
    loop {
        if last_scan.elapsed() >= SCAN_EVERY {
            scan_keyboards(&mut devices);
            last_scan = Instant::now();
        }
        let mut saw = false;
        let mut gone = Vec::new();
        for (path, device) in devices.iter_mut() {
            match device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        saw = true;
                        if let EventSummary::Key(_, code, value) = event.destructure()
                            && combo.push(code, value)
                            && tx.send(()).is_err()
                        {
                            return;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => gone.push(path.clone()),
            }
        }
        for path in gone {
            devices.remove(&path);
        }
        if tx.is_closed() {
            return;
        }
        if !saw {
            std::thread::sleep(IDLE_POLL);
        }
    }
}

fn scan_keyboards(devices: &mut HashMap<PathBuf, Device>) {
    for (path, mut device) in evdev::enumerate() {
        if devices.contains_key(&path) || !is_keyboard(&device) {
            continue;
        }
        if let Err(e) = device.set_nonblocking(true) {
            tracing::debug!(path = %path.display(), error = %e, "companion shortcut: nonblocking failed");
            continue;
        }
        tracing::info!(
            name = %device.name().unwrap_or("unknown"),
            path = %path.display(),
            "companion shortcut listening on keyboard"
        );
        devices.insert(path, device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn default_bind_parses_and_displays() {
        let b = parse_bind(DEFAULT_BIND).unwrap();
        assert!(b.super_key && b.shift && !b.ctrl && !b.alt);
        assert_eq!(b.key, KeyCode::KEY_C);
        assert_eq!(b.to_string(), "Super+Shift+C");
    }

    #[test]
    fn aliases_and_spacing() {
        let a = parse_bind(" meta + shift + c ").unwrap();
        let b = parse_bind("Win+Shift+C").unwrap();
        assert_eq!(a, b);
        assert_eq!(parse_bind("Ctrl+Alt+O").unwrap().to_string(), "Ctrl+Alt+O");
        assert_eq!(parse_bind("Super+F12").unwrap().key, KeyCode::KEY_F12);
    }

    #[test]
    fn rejects_garbage_and_bare_key() {
        assert!(parse_bind("C").is_err());
        assert!(parse_bind("Super").is_err());
        assert!(parse_bind("Super+Shift").is_err());
        assert!(parse_bind("Super+Shift+Nope").is_err());
        assert!(parse_bind("Super+Super+C").is_err());
        assert!(parse_bind("not a bind").is_err());
    }

    #[test]
    fn empty_bind_is_default() {
        assert_eq!(
            parse_bind("").unwrap().to_string(),
            parse_bind(DEFAULT_BIND).unwrap().to_string()
        );
    }

    #[test]
    fn combo_fires_once_on_exact_mods() {
        let bind = parse_bind(DEFAULT_BIND).unwrap();
        let mut s = ComboState::new(bind);
        assert!(!s.push(KeyCode::KEY_LEFTMETA, 1));
        assert!(!s.push(KeyCode::KEY_LEFTSHIFT, 1));
        assert!(s.push(KeyCode::KEY_C, 1));
        assert!(!s.push(KeyCode::KEY_C, 2), "repeat must not re-fire");
        assert!(!s.push(KeyCode::KEY_C, 1), "held key must not re-fire");
        assert!(!s.push(KeyCode::KEY_C, 0));
        assert!(s.push(KeyCode::KEY_C, 1), "release then press fires again");
    }

    #[test]
    fn combo_ignores_wrong_or_extra_mods() {
        let bind = parse_bind(DEFAULT_BIND).unwrap();
        let mut s = ComboState::new(bind);
        assert!(!s.push(KeyCode::KEY_C, 1));
        assert!(!s.push(KeyCode::KEY_LEFTMETA, 1));
        assert!(!s.push(KeyCode::KEY_C, 1), "Super+C is not enough");
        s.push(KeyCode::KEY_C, 0);
        s.push(KeyCode::KEY_LEFTSHIFT, 1);
        s.push(KeyCode::KEY_LEFTCTRL, 1);
        assert!(
            !s.push(KeyCode::KEY_C, 1),
            "extra Ctrl must not match Super+Shift+C"
        );
    }

    #[test]
    fn footer_hint_follows_enabled() {
        let on = OverlayHotkey::default();
        assert_eq!(
            on.footer_hint().as_deref(),
            Some("Hide or show with Super+Shift+C")
        );
        let off = OverlayHotkey {
            enabled: false,
            bind: DEFAULT_BIND.into(),
        };
        assert_eq!(off.footer_hint(), None);
    }

    #[test]
    fn validate_save_rejects_bad_enabled_bind() {
        let bad = OverlayHotkey {
            enabled: true,
            bind: "nope".into(),
        };
        assert!(bad.validate_for_save().is_err());
        let disabled = OverlayHotkey {
            enabled: false,
            bind: "nope".into(),
        };
        assert!(disabled.validate_for_save().is_ok());
        let empty = OverlayHotkey {
            enabled: true,
            bind: "  ".into(),
        };
        assert_eq!(empty.validate_for_save().unwrap().bind, DEFAULT_BIND);
    }

    #[test]
    fn toggle_file_fires_only_on_mtime_advance() {
        let dir = tempfile::tempdir().unwrap();
        let mut last = None;
        assert!(!consume_toggle(dir.path(), &mut last));
        let path = toggle_path(dir.path());
        fs::write(&path, b"").unwrap();
        assert!(
            !consume_toggle(dir.path(), &mut last),
            "first sight of an existing file must not toggle"
        );
        std::thread::sleep(Duration::from_millis(20));
        let t = SystemTime::now() + Duration::from_secs(2);
        let _ = filetime_touch(&path, t);
        assert!(consume_toggle(dir.path(), &mut last));
        assert!(!consume_toggle(dir.path(), &mut last));
    }

    fn filetime_touch(path: &Path, t: SystemTime) -> std::io::Result<()> {
        // Avoid a filetime crate: rewrite so mtime moves.
        let _ = t;
        let mut n = fs::read(path).unwrap_or_default();
        n.push(b'.');
        fs::write(path, n)
    }
}

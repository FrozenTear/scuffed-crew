//! Settings form + screen. Writes today's `Config` fields via `Config::save`.
//!
//! Layout: masonry in 1-4 columns by width (Maps density tokens, G4),
//! Stored data spanning the pane, Save as a full-width footer strip.
//! `capture_route` chooses the screen stream or the older screenshot portal.
//! `data_dir`, `ocr_threads`, and `finished_game_close_secs` are preserved
//! from the loaded file. The extra number reader writes `shadow_recognizer`.
//! The new reader switch writes `reader`. The classic OCR switch writes `ocr`.
//! The alpha updates switch writes `update_channel`.

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Column, Row, button, checkbox, column, container, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Fill, Length, Padding};
use stat_tracker::config::{
    AutoDetectConfig, CaptureRoute, Config, OcrSetting, ShadowRecognizerControl, SyncConfig,
};
use stat_tracker::ocr_switch::{
    self, Audience, PackReadiness, Read as OcrRead, Status as OcrSwitchStatus,
};

use crate::app::{Message, TrackerApp};
use crate::capture_shared::{self, capture_route_help};
use crate::form_widgets::{
    DEMO_NOTE, REPORT_HINT, action_btn, checkbox_style, field_caption, hint, output_chip,
    uninstall_button,
};
use crate::layout::{SCROLLBAR_WIDTH, SETTINGS_SINGLE_MAX, SettingsLayout, settings_layout};
use crate::settings_model::parse_process_names;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAD_INNER, SIZE_BODY, SIZE_LABEL,
    SIZE_META, SIZE_TITLE, TEXT, TEXT_2, TEXT_3,
};
use crate::update;
use crate::widgets;
pub(crate) use stat_tracker::capture::SessionKind;

/// Seconds / counts, must stay compact on ultrawide.
pub(crate) const FIELD_NUMERIC: f32 = 168.0;
/// Shortcut, in-game name.
pub(crate) const FIELD_SHORT: f32 = 280.0;
/// URL, token, process list. Capped so they do not become a full-pane bar.
pub(crate) const FIELD_TEXT: f32 = 480.0;

/// Help copy follows the card up to this width (about 130 characters at
/// `SIZE_LABEL`). At 520 px the two-line hints on a 760-790 px card ended in
/// a line of one or two words.
pub(crate) const HELP_MEASURE: f32 = 640.0;
/// Widest a "text … button" row gets inside a card that spans the pane
/// (Report a problem, Uninstall, the Save strip). Past this the button was
/// 2200-3000 px from its label on a wide window. It equals the one-column
/// pane cap, so those rows are unchanged on a narrow or portrait window.
pub(crate) const ACTION_ROW_MAX: f32 = SETTINGS_SINGLE_MAX;

const _: () = {
    assert!(FIELD_TEXT <= HELP_MEASURE);
    assert!(HELP_MEASURE < crate::layout::MAX_SETTINGS_CARD - 2.0 * PAD_INNER);
    assert!(FIELD_NUMERIC <= 200.0);
    assert!(FIELD_SHORT < 400.0);
    assert!(FIELD_TEXT <= 560.0);
    assert!(FIELD_NUMERIC < FIELD_SHORT);
    assert!(FIELD_SHORT < FIELD_TEXT);
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    PlayerName,
    SessionWindow,
    ProcessNames,
    PollInterval,
    Cooldown,
    SyncUrl,
    SyncToken,
    OverlayHotkey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsToggle {
    AutoDetect,
    DebugOcr,
    OverlayHotkey,
    ShadowRecognizer,
    NewReader,
    /// "Use classic OCR (Tesseract)". The app checks the reader packs before
    /// it lets this go off ([`classic_ocr_toggle`]).
    ClassicOcr,
    PrereleaseUpdates,
}

/// Scoreboard number reader the Settings control can select.
///
/// Today the control is one checkbox. Off is [`Self::OcrV1`] (nothing extra
/// is stored). On is [`Self::Extra`], which writes `shadow_recognizer = true`
/// and runs the shadow test reader as a private log. The extra reader's id
/// is [`stat_tracker::shadow::digits::RECOGNIZER_ID`], so the next bump of
/// that constant shows up here. A later picker can list [`Self::choices`]
/// in this order and still write the same bool until the config key itself
/// becomes an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreboardReader {
    /// Default. Saved games use ocr-v1. The extra reader is off.
    OcrV1,
    /// Extra test reader. Id is the shadow matcher's `RECOGNIZER_ID`.
    Extra,
}

impl ScoreboardReader {
    pub const OCR_V1: &'static str = "ocr-v1";

    pub const fn id(self) -> &'static str {
        match self {
            Self::OcrV1 => Self::OCR_V1,
            Self::Extra => stat_tracker::shadow::digits::RECOGNIZER_ID,
        }
    }

    /// Display order for a later picker. The checkbox uses the ends of this list.
    pub const fn choices() -> [Self; 2] {
        [Self::OcrV1, Self::Extra]
    }

    pub const fn from_file_flag(on: bool) -> Self {
        if on { Self::Extra } else { Self::OcrV1 }
    }

    pub const fn file_flag(self) -> bool {
        matches!(self, Self::Extra)
    }
}

pub(crate) const SHADOW_READER_LABEL: &str = "Extra number reader (test)";

pub(crate) const SHADOW_READER_HINT: &str = "\
It only writes a private log on this computer. \
It doesn't change saved games or uploads, and it's off by default.";

/// The daemon reads config once at startup. This setting does not apply live.
pub(crate) const SHADOW_READER_RESTART: &str =
    "The tracker reads this when it starts. Restart it after you save.";

pub(crate) const SHADOW_READER_ENV_NOTE: &str = "\
SCUFFED_SHADOW_RECOGNIZER is set for this app, so the toggle stays on and is not written to the config file. \
The tracker service may differ.";

pub(crate) const NEW_READER_LABEL: &str = "New reader";

pub(crate) const NEW_READER_HINT: &str = "\
Saved games and uploads use the new scoreboard reader. \
With classic OCR on, a field it is not sure about keeps the classic read and is marked unsure. \
With classic OCR off, a stat it is not sure about waits for a later Tab that reads it clearly. If none does, the game is saved with its best guess and that stat is marked unsure.";

pub(crate) const NEW_READER_RESTART: &str =
    "The tracker reads this when it starts. Restart it after you save.";

pub(crate) const CLASSIC_OCR_LABEL: &str = "Use classic OCR (Tesseract)";

/// The hint while this tracker version can turn classic OCR off. It says
/// what the member gives up.
///
/// * The map on the scoreboard has no second reader. That has two effects,
///   and both are named: a game whose map is never read has no map, and a
///   co-op event board is told apart only when its map name is read sure.
/// * The names are not read, so one kind of scoreboard has no way left to
///   find the member's row.
/// * The new reader, which off implies, stores no game that ends with a
///   stat it never read.
pub(crate) const CLASSIC_OCR_HINT: &str = "\
Tesseract is the classic text reader. Off: the tracker reads the game with its own reader \
and does not start Tesseract. With classic OCR off the map name on the scoreboard has no \
second reader: a game whose map is never read is saved without a map until you pick one, \
and a co-op event scoreboard (Junkenstein) can be saved as a game when its map name is not \
read. A scoreboard is not saved when your row has never been highlighted, your hero icon \
is dead or covered, and the tracker has not learned your name yet. A game that ends with \
a stat never read is not saved.";

/// The hint while this tracker version still needs classic OCR for a read:
/// no build of it turns the switch off, so nothing is promised.
pub(crate) const CLASSIC_OCR_HINT_NOT_YET: &str = "\
Tesseract is the classic text reader. This tracker version still needs it for some reads, \
so it stays on.";

/// The button that takes a shown refusal away.
pub(crate) const DISMISS_REFUSAL_LABEL: &str = "OK";

pub(crate) const CLASSIC_OCR_RESTART: &str =
    "The tracker reads this when it starts. Restart it after you save.";

/// Shown in place of the "New reader" switch while the tracker runs
/// with classic OCR off.
pub(crate) const NEW_READER_IMPLIED: &str =
    "The tracker is running with classic OCR off, so it always uses the new reader.";

pub(crate) const READER_PACKS_UNCHECKED: &str = "Reader packs: checking…";

/// The OCR switch as Settings shows it. Not part of the form: none of it is
/// saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OcrSwitchView {
    /// The reader packs in the data dir, once they were looked at.
    pub packs: Option<PackReadiness>,
    /// Why the last click to turn classic OCR off was refused.
    pub refusal: Option<String>,
    /// What the running tracker reports: it kept OCR on, or it runs with
    /// OCR off and holds reads for a missing pack.
    pub notice: Option<String>,
    /// The running tracker's notice for Tracker health
    /// ([`daemon_health_notice`]): a hold or a stopped read, never a
    /// refused off.
    pub health_notice: Option<String>,
    /// The running tracker decided to run with classic OCR off
    /// ([`daemon_runs_ocr_off`]). The card hides what only classic OCR uses
    /// when this is true, not when the box is merely unticked.
    pub daemon_off: bool,
    /// Why captures are stopped after a declined screen share. Shown in
    /// Tracker health, not mixed into the OCR notice.
    pub share_notice: Option<String>,
    /// Why the running tracker is not on the capture method asked for
    /// (KDE direct did not answer). Shown under the capture method.
    pub capture_notice: Option<String>,
}

/// A click on the classic OCR switch. Turning it on always works. Turning
/// it off needs a tracker version that reads everything without Tesseract
/// (`needing_ocr` empty) and every reader pack: `Err` is the reason shown
/// next to the switch, and the switch stays on. `Ok` is the new switch
/// state.
pub(crate) fn classic_ocr_toggle(
    want_classic: bool,
    packs: &PackReadiness,
    needing_ocr: &[OcrRead],
) -> Result<bool, String> {
    if want_classic {
        return Ok(true);
    }
    match ocr_switch::decide(OcrSetting::Off, packs, needing_ocr).refusal {
        Some(refusal) => Err(refusal.text(Audience::Click)),
        None => Ok(false),
    }
}

/// Why a save that turns classic OCR off must not be written. Checked only
/// for that change, against the packs as they were last looked at (`None`
/// before the first look: the tracker itself refuses at start then).
pub(crate) fn ocr_save_problem(
    saved: OcrSetting,
    form_classic: bool,
    packs: Option<&PackReadiness>,
    needing_ocr: &[OcrRead],
) -> Option<String> {
    if saved.is_off() || form_classic {
        return None;
    }
    classic_ocr_toggle(false, packs?, needing_ocr).err()
}

/// The refusal still shown next to the switch after the packs were looked
/// at again: it goes away once the click would be granted.
pub(crate) fn refusal_after_pack_check(
    shown: Option<String>,
    packs: &PackReadiness,
    needing_ocr: &[OcrRead],
) -> Option<String> {
    shown.and_then(|_| classic_ocr_toggle(false, packs, needing_ocr).err())
}

/// The running tracker's status. `None` when the status file is from
/// another run than the tracker that is up now (or none is).
fn live_status(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<&OcrSwitchStatus> {
    status.filter(|status| daemon_pid == Some(status.pid))
}

/// The running tracker's notice about the switch ([`live_status`]).
pub(crate) fn daemon_ocr_notice(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<String> {
    live_status(status, daemon_pid)?.notice()
}

/// True when the running tracker decided to run with classic OCR off. Not
/// the config value: a tracker that refused off, or one that was not
/// restarted since the save, still runs Tesseract.
pub(crate) fn daemon_runs_ocr_off(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> bool {
    live_status(status, daemon_pid).is_some_and(|status| status.effective.is_off())
}

/// The tracker's notice for Tracker health: only what is wrong while it
/// runs with classic OCR off. A refused off is not a health problem (the
/// tracker reads everything with classic OCR), so health stays "Ready" and
/// the reason is shown in Settings ([`daemon_ocr_notice`]).
pub(crate) fn daemon_health_notice(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<String> {
    live_status(status, daemon_pid)?.health_notice()
}

/// The running tracker's declined-share sentence, for Tracker health.
/// Empty, or a status file from another run, is nothing to show.
pub(crate) fn daemon_share_notice(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<String> {
    let notice = live_status(status, daemon_pid)?.share_notice.trim();
    if notice.is_empty() {
        None
    } else {
        Some(notice.to_string())
    }
}

/// The running tracker's capture-route sentence ([`OcrSwitchStatus::capture_notice`]).
pub(crate) fn daemon_capture_notice(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<String> {
    let notice = live_status(status, daemon_pid)?.capture_notice.trim();
    (!notice.is_empty()).then(|| notice.to_string())
}

/// The running tracker's screen share is open ([`live_status`]).
pub(crate) fn daemon_share_open(status: Option<&OcrSwitchStatus>, daemon_pid: Option<u32>) -> bool {
    live_status(status, daemon_pid).is_some_and(|status| status.share_open)
}

/// KDE direct is checked again when the tracker restarted (a new pid,
/// after a capture route change) or its screen share just closed. `seen`
/// is (tracker pid, share open) at the last tick and at this one.
pub(crate) fn kde_direct_recheck_due(
    before: (Option<u32>, bool),
    now: (Option<u32>, bool),
) -> bool {
    let restarted = now.0.is_some() && now.0 != before.0;
    let share_closed = before.1 && !now.1;
    restarted || share_closed
}

/// Shown under the capture method when KDE direct can be chosen.
pub(crate) const KDE_DIRECT_HELP: &str = "KDE direct asks KWin directly; Plasma shows no prompt or indicator; images stay in memory and are never saved unless a debug option is on. Needs Plasma 6.8 or later.";

/// KDE direct has not been tried on this desktop yet.
pub(crate) const KDE_DIRECT_NOT_CHECKED: &str =
    "KDE direct has not been checked here yet. Choosing it runs a short check.";

/// Shown on GNOME next to the Older screenshot choice.
pub(crate) const GNOME_SCREENSHOT_NOTE: &str =
    "Older screenshot: GNOME flashes the screen and plays a sound on every capture.";

/// How a line under the capture method is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteTone {
    Info,
    Warn,
}

/// Whether the app may run KDE direct's 1x1 KWin render: only when KDE
/// direct is the saved route, or automatic is saved and automatic would
/// use KDE direct ([`stat_tracker::capture::KWIN_DIRECT_DEFAULT_ON_KDE`]),
/// or the user just chose its chip. Otherwise Settings reads the version
/// only, which renders nothing.
pub(crate) fn render_wanted(saved_route: CaptureRoute, chip_clicked: bool) -> bool {
    render_wanted_with(
        saved_route,
        chip_clicked,
        stat_tracker::capture::KWIN_DIRECT_DEFAULT_ON_KDE,
    )
}

fn render_wanted_with(saved_route: CaptureRoute, chip_clicked: bool, default_on: bool) -> bool {
    saved_route == CaptureRoute::Kwin
        || (saved_route == CaptureRoute::Auto && default_on)
        || chip_clicked
}

/// The install / update row in the Tracker service card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallRow {
    pub caption: &'static str,
    pub line: String,
    /// Whether `line` is a command with a Copy button.
    pub copy: bool,
}

/// A package install is told to update with its package manager, with no
/// command to copy: the curl installer would add a second install in
/// `~/.local` that shadows the package (S5-UP-2). Until the package probe
/// answers there is no command yet. Any other install gets the pinned
/// install command, as before.
pub(crate) fn install_row(
    package: update::PackageProbe<'_>,
    advertised: Option<&str>,
    current: Option<&str>,
) -> InstallRow {
    match package {
        update::PackageProbe::Done(Some(owner)) => InstallRow {
            caption: "Updates",
            line: update::package_update_line(owner),
            copy: false,
        },
        update::PackageProbe::Pending => InstallRow {
            caption: "Install / update command",
            line: update::PACKAGE_PROBE_PENDING.to_string(),
            copy: false,
        },
        update::PackageProbe::Done(None) => InstallRow {
            caption: "Install / update command",
            line: update::install_command_for(advertised, current),
            copy: true,
        },
    }
}

/// The KDE direct chip is shown on KDE, or when it is the saved choice.
pub(crate) fn show_kde_direct(desktop: &str, route: CaptureRoute) -> bool {
    stat_tracker::capture::is_kde(desktop) || route == CaptureRoute::Kwin
}

/// The session as the tracker sees it ([`SessionKind::from_env`], its
/// three-state rule) for this process's environment. Settings hides the
/// Wayland-only capture methods only on [`SessionKind::X11`], where the
/// tracker ignores them too. [`SessionKind::Unknown`] (no `WAYLAND_DISPLAY`
/// and a type that is unset, `tty` or `unspecified`, or `x11` beside a
/// stray `WAYLAND_DISPLAY`) renders like rc.7: every chip, no warning.
pub(crate) fn session_from_env() -> SessionKind {
    SessionKind::from_env(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
    )
}

/// Capture methods that need a Wayland session: the screen stream and KDE
/// direct. Older screenshot (the portal) also runs on X11.
pub(crate) fn wayland_only_route(route: CaptureRoute) -> bool {
    matches!(route, CaptureRoute::Screencast | CaptureRoute::Kwin)
}

/// The capture method chips, in order: Automatic, Screen stream, Older
/// screenshot, and KDE direct on KDE or when it is saved. On a session
/// known to be X11 the Wayland-only methods are left out, except a saved
/// one, which stays (selected) so the warning under it has a chip to point
/// at. Wayland and an unknown session get every chip.
pub(crate) fn capture_route_chips(
    desktop: &str,
    session: SessionKind,
    route: CaptureRoute,
) -> Vec<CaptureRoute> {
    let mut chips = vec![
        CaptureRoute::Auto,
        CaptureRoute::Screencast,
        CaptureRoute::Screenshot,
    ];
    if show_kde_direct(desktop, route) {
        chips.push(CaptureRoute::Kwin);
    }
    if session == SessionKind::X11 {
        chips.retain(|chip| !wayland_only_route(*chip) || *chip == route);
    }
    chips
}

pub(crate) fn capture_route_label(route: CaptureRoute) -> &'static str {
    match route {
        CaptureRoute::Auto => "Automatic",
        CaptureRoute::Screencast => "Screen stream",
        CaptureRoute::Screenshot => "Older screenshot",
        CaptureRoute::Kwin => "KDE direct",
    }
}

/// Shown on X11 next to the Older screenshot choice.
pub(crate) const X11_SCREENSHOT_NOTE: &str = "Older screenshot captures the whole desktop on X11 with more than one monitor, so Automatic is recommended.";

/// The warning under a saved Wayland-only method on X11.
pub(crate) fn x11_route_warning(route: CaptureRoute) -> String {
    format!(
        "{} works only on Wayland. This is an X11 session, so the tracker uses Automatic instead.",
        capture_route_label(route)
    )
}

/// The tracker's capture notice as a Settings line. A notice that already
/// ends a sentence (the screenshot portal's) keeps one period, not two.
fn tracker_note(notice: &str) -> String {
    let notice = notice.trim_end();
    format!("Tracker: {}.", notice.strip_suffix('.').unwrap_or(notice))
}

/// Lines under the capture method: a saved Wayland-only method on X11,
/// Older screenshot on X11, the GNOME flash for Older screenshot, KDE
/// direct that is not there, and what the running tracker said. A line
/// about the selected choice is a warning. On X11 the KDE direct lines are
/// left out (it is not offered there), and so is the tracker's notice about
/// the same Wayland-only method, which the warning already says.
pub(crate) fn capture_route_notes(
    desktop: &str,
    session: SessionKind,
    route: CaptureRoute,
    kde_direct: Option<&stat_tracker::capture::KwinProbe>,
    daemon_notice: Option<&str>,
) -> Vec<(String, NoteTone)> {
    let tone = |selected: bool| {
        if selected {
            NoteTone::Warn
        } else {
            NoteTone::Info
        }
    };
    let mut notes = Vec::new();
    if session == SessionKind::X11 {
        let warned = wayland_only_route(route);
        if warned {
            notes.push((x11_route_warning(route), NoteTone::Warn));
        }
        notes.push((
            X11_SCREENSHOT_NOTE.to_string(),
            tone(route == CaptureRoute::Screenshot),
        ));
        if stat_tracker::capture::is_gnome(desktop) {
            notes.push((
                GNOME_SCREENSHOT_NOTE.to_string(),
                tone(route == CaptureRoute::Screenshot),
            ));
        }
        // The tracker's notice about the same saved method is the warning
        // above, unless it adds that nothing else works here.
        if let Some(notice) = daemon_notice
            && !(warned
                && notice.contains("needs a Wayland session")
                && !notice.contains("no other capture method works here"))
        {
            notes.push((tracker_note(notice), NoteTone::Warn));
        }
        return notes;
    }
    if stat_tracker::capture::is_gnome(desktop) {
        notes.push((
            GNOME_SCREENSHOT_NOTE.to_string(),
            tone(route == CaptureRoute::Screenshot),
        ));
    }
    match kde_direct {
        Some(stat_tracker::capture::KwinProbe::Unavailable(reason)) => notes.push((
            format!("KDE direct is not available here: {reason}."),
            tone(route == CaptureRoute::Kwin),
        )),
        // Not a verdict: said plainly, never as a warning.
        Some(stat_tracker::capture::KwinProbe::NotChecked(reason)) => notes.push((
            format!("KDE direct was not checked: {reason}."),
            NoteTone::Info,
        )),
        Some(stat_tracker::capture::KwinProbe::NotProbed) => {
            notes.push((KDE_DIRECT_NOT_CHECKED.to_string(), NoteTone::Info));
        }
        Some(stat_tracker::capture::KwinProbe::Available) | None => {}
    }
    if let Some(notice) = daemon_notice {
        notes.push((tracker_note(notice), NoteTone::Warn));
    }
    notes
}

/// The lines under the classic OCR switch that depend on the tracker
/// version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OcrHintLines {
    pub hint: &'static str,
    /// "The tracker reads this when it starts" and the reader pack count:
    /// only where the switch can be turned off at all.
    pub restart_and_packs: bool,
}

/// `needing_ocr` is what this tracker version still needs classic OCR for.
/// While that is anything, the card does not describe an off no build
/// gives.
pub(crate) fn ocr_hint_lines(needing_ocr: &[OcrRead]) -> OcrHintLines {
    if needing_ocr.is_empty() {
        OcrHintLines {
            hint: CLASSIC_OCR_HINT,
            restart_and_packs: true,
        }
    } else {
        OcrHintLines {
            hint: CLASSIC_OCR_HINT_NOT_YET,
            restart_and_packs: false,
        }
    }
}

/// The one problem line under the switch: the refused click when there is
/// one (it can be dismissed), else the running tracker's notice. Never
/// both: after a hand-edited `off` they are the same sentence.
pub(crate) fn card_problem<'a>(
    refusal: Option<&'a str>,
    notice: Option<&'a str>,
) -> Option<(&'a str, bool)> {
    match (refusal, notice) {
        (Some(refusal), _) => Some((refusal, true)),
        (None, Some(notice)) => Some((notice, false)),
        (None, None) => None,
    }
}

/// What both pages show under the reader controls: the problem line drawn
/// by `warn` (with OK when it is a refused click), then the "new reader"
/// hint while the running tracker has classic OCR off.
pub(crate) fn ocr_problem_lines<'a>(
    app: &'a TrackerApp,
    warn: impl Fn(&'a str) -> Element<'a, Message>,
) -> Vec<Element<'a, Message>> {
    let mut lines = Vec::new();
    if let Some((problem, dismissable)) = card_problem(
        app.ocr_switch.refusal.as_deref(),
        app.ocr_switch.notice.as_deref(),
    ) {
        lines.push(warn(problem));
        if dismissable {
            lines.push(action_btn(
                DISMISS_REFUSAL_LABEL,
                false,
                Message::DismissOcrRefusal,
            ));
        }
    }
    if app.ocr_switch.daemon_off {
        lines.push(hint(NEW_READER_IMPLIED));
    }
    lines
}

/// "Reader packs ready: 2 of 3", or a placeholder until they were checked.
pub(crate) fn pack_status_line(packs: Option<&PackReadiness>) -> String {
    packs.map_or_else(
        || READER_PACKS_UNCHECKED.to_string(),
        PackReadiness::status_line,
    )
}

/// Which parts of the Scoreboard reading card are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OcrCardBlocks {
    /// "Install reading model" and the model
    /// status. Tesseract's model: of no use to a tracker that runs with
    /// classic OCR off.
    pub reading_model: bool,
    /// The "New reader" switch. Off implies the new reader, so the
    /// switch has nothing to choose.
    pub new_reader: bool,
}

/// `daemon_off` is [`OcrSwitchView::daemon_off`]. While the tracker runs
/// Tesseract (the box unticked but off refused, or not restarted yet, or no
/// tracker running), both blocks stay: the member may need the model.
pub(crate) fn ocr_card_blocks(daemon_off: bool) -> OcrCardBlocks {
    OcrCardBlocks {
        reading_model: !daemon_off,
        new_reader: !daemon_off,
    }
}

pub(crate) const PRERELEASE_UPDATES_LABEL: &str = "Get alpha updates";

pub(crate) const PRERELEASE_UPDATES_HINT: &str = "\
Offer test builds (alphas and release candidates) as updates. They may have bugs. \
Turn off to get only stable releases.";

/// Only this app reads `update_channel`. Save checks for updates again.
pub(crate) const PRERELEASE_UPDATES_APPLIES: &str =
    "This app checks again when you save. No restart needed.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsForm {
    pub capture_output: String,
    /// `capture_route`. Automatic, the screen stream, or the older screenshot.
    pub capture_route: CaptureRoute,
    pub player_name: String,
    pub session_window_secs: String,
    pub game_process_names: String,
    pub auto_detect_enabled: bool,
    pub poll_interval_secs: String,
    pub cooldown_secs: String,
    pub sync_url: String,
    pub sync_token: String,
    pub debug_ocr: bool,
    pub overlay_hotkey: String,
    pub overlay_hotkey_enabled: bool,
    /// File value and env lock for the extra number reader.
    /// Save writes `file_on` only.
    pub shadow: ShadowRecognizerControl,
    /// `reader = "new"`. Independent of the extra-log checkbox.
    pub new_reader: bool,
    /// `ocr = "on"`. False writes `ocr = "off"`. `reader` keeps its value.
    pub classic_ocr: bool,
    /// "Get alpha updates" checkbox. Shows the channel the update check
    /// uses, so an alpha or rc install with `update_channel` unset shows it
    /// ticked. Read by this app's update check only.
    pub prerelease_updates: bool,
    /// What the checkbox showed when the form was loaded. Save writes
    /// `update_channel` only when `prerelease_updates` differs from this.
    pub prerelease_updates_shown: bool,
}

impl Default for SettingsForm {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

impl SettingsForm {
    pub fn from_config(config: &Config) -> Self {
        Self::from_config_and_shadow(config, config.shadow_recognizer_control())
    }

    /// [`Self::from_config`] with an explicit shadow control.
    ///
    /// Tests pass this so they do not read `SCUFFED_SHADOW_RECOGNIZER` from
    /// the process. `shadow.file_on` is the value Save writes. The app passes
    /// [`Config::shadow_recognizer_control`], whose `file_on` is the file.
    pub fn from_config_and_shadow(config: &Config, shadow: ShadowRecognizerControl) -> Self {
        Self {
            capture_output: config.capture_output.clone().unwrap_or_default(),
            capture_route: config.capture_route,
            player_name: config.player_name.clone().unwrap_or_default(),
            session_window_secs: config.session_window_secs.to_string(),
            game_process_names: config.game_process_names.join(", "),
            auto_detect_enabled: config.auto_detect.enabled,
            // Empty means automatic (1.5 s new reader, 4 s classic OCR).
            // 4 is how automatic is written to the file
            // (`AutoDetectConfig::poll_interval`), so it shows as empty.
            poll_interval_secs: config
                .auto_detect
                .poll_interval_secs
                .filter(|&s| s != stat_tracker::config::LEGACY_DEFAULT_POLL_SECS)
                .map(|s| s.to_string())
                .unwrap_or_default(),
            cooldown_secs: config.auto_detect.cooldown_secs.to_string(),
            sync_url: config
                .sync
                .as_ref()
                .map(|s| s.server_url.clone())
                .unwrap_or_default(),
            sync_token: config
                .sync
                .as_ref()
                .map(|s| s.token.clone())
                .unwrap_or_default(),
            debug_ocr: config.debug_ocr,
            overlay_hotkey: crate::hotkey::DEFAULT_BIND.to_string(),
            overlay_hotkey_enabled: true,
            shadow,
            new_reader: config.reader.is_new(),
            classic_ocr: config.ocr.is_on(),
            prerelease_updates: config.wants_prerelease_updates(),
            prerelease_updates_shown: config.wants_prerelease_updates(),
        }
    }

    /// Show the alpha updates checkbox for the installed build. With
    /// `update_channel` unset, an alpha or rc install is on the prerelease
    /// channel (see [`crate::update::effective_update_channel`]), so the box
    /// shows ticked. Ticking or unticking still writes the key, and leaving
    /// it alone writes nothing.
    pub fn with_installed_version(mut self, config: &Config, installed: Option<&str>) -> Self {
        let on = crate::update::prerelease_updates_effective(&config.update_channel, installed);
        self.prerelease_updates = on;
        self.prerelease_updates_shown = on;
        self
    }

    /// Checkbox state. The env lock shows the extra reader even when the file is off.
    pub fn displayed_reader(&self) -> ScoreboardReader {
        if self.shadow.enabled() {
            ScoreboardReader::Extra
        } else {
            ScoreboardReader::OcrV1
        }
    }

    /// Value Save writes. Never the env override.
    pub fn persisted_reader(&self) -> ScoreboardReader {
        ScoreboardReader::from_file_flag(self.shadow.file_on)
    }

    /// Picker seam. Ignored while the env lock is on, so a click cannot
    /// write the override into config.toml.
    pub fn set_scoreboard_reader(&mut self, reader: ScoreboardReader) {
        if self.shadow.locked {
            return;
        }
        self.shadow.file_on = reader.file_flag();
    }

    pub fn set_text(&mut self, field: SettingsField, value: String) {
        match field {
            SettingsField::PlayerName => self.player_name = value,
            SettingsField::SessionWindow => self.session_window_secs = value,
            SettingsField::ProcessNames => self.game_process_names = value,
            SettingsField::PollInterval => self.poll_interval_secs = value,
            SettingsField::Cooldown => self.cooldown_secs = value,
            SettingsField::SyncUrl => self.sync_url = value,
            SettingsField::SyncToken => self.sync_token = value,
            SettingsField::OverlayHotkey => self.overlay_hotkey = value,
        }
    }

    pub fn set_toggle(&mut self, toggle: SettingsToggle, value: bool) {
        match toggle {
            SettingsToggle::AutoDetect => self.auto_detect_enabled = value,
            SettingsToggle::DebugOcr => self.debug_ocr = value,
            SettingsToggle::OverlayHotkey => self.overlay_hotkey_enabled = value,
            SettingsToggle::ShadowRecognizer => {
                self.set_scoreboard_reader(ScoreboardReader::from_file_flag(value));
            }
            SettingsToggle::NewReader => self.new_reader = value,
            SettingsToggle::ClassicOcr => self.classic_ocr = value,
            SettingsToggle::PrereleaseUpdates => self.prerelease_updates = value,
        }
    }

    /// Map the form onto `base`, keeping `data_dir`, `ocr_threads`, the report kit caps,
    /// `finished_game_close_secs`, `setup_completed`, `reader_pack_url`,
    /// `debug_portal_frames`, the stream keys, and `name_reader` (the 0.6
    /// page sets those through [`crate::settings_model::ConfigEdit`]).
    /// `shadow_recognizer` comes from the form's persisted reader (the file
    /// value). An env lock never writes true by itself. `update_channel`
    /// keeps the file's value (unset stays unset) unless the checkbox was
    /// flipped, then it becomes `prerelease` or `stable`.
    pub fn to_config(&self, base: &Config) -> Config {
        Config {
            data_dir: base.data_dir.clone(),
            capture_output: nonempty(&self.capture_output),
            capture_route: self.capture_route,
            debug_portal_frames: base.debug_portal_frames,
            screencast_max_fps: base.screencast_max_fps,
            screencast_pause_between_grabs: base.screencast_pause_between_grabs,
            player_name: nonempty(&self.player_name),
            sync: sync_from_fields(&self.sync_url, &self.sync_token),
            auto_detect: AutoDetectConfig {
                enabled: self.auto_detect_enabled,
                poll_interval_secs: self
                    .poll_interval_secs
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|&s| s > 0)
                    .or(base
                        .auto_detect
                        .poll_interval_secs
                        .filter(|&s| s == stat_tracker::config::LEGACY_DEFAULT_POLL_SECS)),
                poll_interval_ms: base.auto_detect.poll_interval_ms,
                cooldown_secs: parse_u64(&self.cooldown_secs, 120),
            },
            session_window_secs: parse_u64(&self.session_window_secs, 1800),
            finished_game_close_secs: base.finished_game_close_secs,
            game_process_names: parse_process_names(&self.game_process_names),
            debug_ocr: self.debug_ocr,
            report_kit_max_games: base.report_kit_max_games,
            report_kit_max_mb: base.report_kit_max_mb,
            ocr_threads: base.ocr_threads,
            shadow_recognizer: self.persisted_reader().file_flag(),
            update_channel: update_channel_for(
                base,
                self.prerelease_updates_shown,
                self.prerelease_updates,
            ),
            setup_completed: base.setup_completed,
            reader_pack_url: base.reader_pack_url.clone(),
            name_reader: base.name_reader,
            reader: if self.new_reader {
                stat_tracker::config::ReaderSetting::New
            } else {
                stat_tracker::config::ReaderSetting::OcrV1
            },
            ocr: if self.classic_ocr {
                OcrSetting::On
            } else {
                OcrSetting::Off
            },
        }
    }
}

/// `update_channel` to save. A checkbox still showing what it showed at load
/// keeps `base`'s raw value, so a save that touches nothing else leaves the
/// file byte for byte and never writes `stable` on its own.
pub fn update_channel_for(base: &Config, shown: bool, prerelease: bool) -> String {
    if prerelease == shown {
        base.update_channel.clone()
    } else if prerelease {
        "prerelease".to_string()
    } else {
        "stable".to_string()
    }
}

pub fn nonempty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Both URL and token are required, same as the Dioxus save path.
/// An unsafe URL yields `None` so a config value cannot carry a token that
/// the daemon would have to refuse. Save itself is blocked first
/// ([`sync_url_problem`]) so an existing cleartext URL is not wiped.
pub fn sync_from_fields(url: &str, token: &str) -> Option<SyncConfig> {
    let server_url = url.trim();
    let token = token.trim();
    if server_url.is_empty() || token.is_empty() || sync_url_problem(server_url).is_some() {
        None
    } else {
        Some(SyncConfig {
            server_url: server_url.to_string(),
            token: token.to_string(),
        })
    }
}

/// Inline Settings warning. `None` when the field is blank (sync off) or the
/// URL is safe to save. Save must not persist a URL that returns `Some`.
pub fn sync_url_problem(url: &str) -> Option<&'static str> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    stat_tracker::config::validate_sync_server_url(url)
        .err()
        .map(|e| e.message())
}

pub fn parse_u64(raw: &str, fallback: u64) -> u64 {
    raw.trim().parse().unwrap_or(fallback)
}

pub fn save_config(config: &Config) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

/// Layout-only width cap. Save / bind behaviour is unchanged.
pub(crate) fn field_max_width(field: SettingsField) -> f32 {
    match field {
        SettingsField::SessionWindow | SettingsField::PollInterval | SettingsField::Cooldown => {
            FIELD_NUMERIC
        }
        SettingsField::PlayerName | SettingsField::OverlayHotkey => FIELD_SHORT,
        SettingsField::ProcessNames | SettingsField::SyncUrl | SettingsField::SyncToken => {
            FIELD_TEXT
        }
    }
}

/// Share of a [`compact_row`] this field's column takes. Iced makes each
/// `Fill` child of a row exactly its share wide (a max width on the column
/// does nothing there), so the share is what the hint wraps at; the input
/// keeps its own cap inside the column. The in-game name has the longest
/// hint (about 243 px) next to the session window's shorter one (about
/// 178 px), so it takes 4 parts to 3. An even split left it 241 px on a 1280
/// window and the hint wrapped a lone "#1234." onto a second line.
pub(crate) fn field_fill_portion(field: SettingsField) -> u16 {
    match field {
        SettingsField::PlayerName => 4,
        _ => 3,
    }
}

/// Every settings card. Order here is the 1-column stack and the masonry input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsSection {
    Daemon,
    Capture,
    Companion,
    Player,
    AutoDetect,
    Sync,
    Ocr,
    Diagnostics,
    Data,
}

/// Masonry input. Stored data spans the pane so the last row is not
/// a skyscraper + empty pocket.
pub(crate) const MASONRY_SECTIONS: &[SettingsSection] = &[
    SettingsSection::Daemon,
    SettingsSection::Capture,
    SettingsSection::Companion,
    SettingsSection::Player,
    SettingsSection::AutoDetect,
    SettingsSection::Sync,
    SettingsSection::Ocr,
    SettingsSection::Diagnostics,
];

pub(crate) const SPANNING_SECTION: SettingsSection = SettingsSection::Data;
pub(crate) const SAVE_LABEL: &str = "Save settings";

/// Relative visual height after caption tightening. Used only to pack columns.
pub(crate) fn section_weight(section: SettingsSection) -> u16 {
    match section {
        SettingsSection::Daemon => 3,
        SettingsSection::Capture => 3,
        SettingsSection::Companion => 3,
        SettingsSection::Player => 4,
        SettingsSection::AutoDetect => 3,
        SettingsSection::Sync => 3,
        SettingsSection::Ocr => 3,
        SettingsSection::Diagnostics => 2,
        SettingsSection::Data => 3,
    }
}

/// Greedy masonry for any column count: each section goes into the currently
/// shortest column. Tie → leftmost. One column returns the input order.
/// Never more columns than sections, so no column is left empty.
pub(crate) fn pack_columns(sections: &[SettingsSection], cols: usize) -> Vec<Vec<SettingsSection>> {
    let cols = cols.clamp(1, sections.len().max(1));
    if cols == 1 {
        return vec![sections.to_vec()];
    }
    let mut columns = vec![Vec::new(); cols];
    let mut heights = vec![0u16; cols];
    for &section in sections {
        let i = heights
            .iter()
            .enumerate()
            .min_by_key(|&(i, h)| (*h, i))
            .map(|(i, _)| i)
            .unwrap_or(0);
        columns[i].push(section);
        heights[i] = heights[i].saturating_add(section_weight(section));
    }
    columns
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let demo = app.fixture.is_some();
    let layout = settings_layout(content_width);
    let cols = layout.columns;
    let mut col = column![].spacing(GRID_GAP).width(Fill);

    if let Some(info) = &app.update
        && app.update_banner_visible()
    {
        col = col.push(update::banner(
            info,
            &app.update_progress,
            &app.update_plan,
            &app.update_notes,
            app.update_notes_ui(),
            app.update_notes_expanded,
            app.restart_need(),
            app.package_probe(),
        ));
    }

    if demo {
        col = col.push(widgets::empty_surface(DEMO_NOTE));
    }

    col = col.push(action_btn(
        "Open setup guide",
        false,
        Message::Setup(crate::setup_guide::SetupMessage::Open),
    ));

    let packed = pack_columns(MASONRY_SECTIONS, cols);
    let columns: Vec<Vec<Element<'_, Message>>> = packed
        .into_iter()
        .map(|ids| {
            ids.into_iter()
                .map(|id| section_card(id, app, demo))
                .collect()
        })
        .collect();
    col = col
        .push(section_columns(columns))
        .push(section_card(SPANNING_SECTION, app, demo))
        .push(save_footer(demo))
        .push(report_row())
        .push(uninstall_row());

    scroll_frame(layout, col)
}

/// The scrolling settings pane. The body is left-aligned and stops at
/// `layout.pane`, which leaves the scrollbar gutter free at the right
/// whether or not the bar shows, so the cards do not move when the page
/// starts to scroll. (`Scrollable::spacing` only reserves the space while
/// the bar shows, and the column count could not follow that.)
fn scroll_frame<'a>(layout: SettingsLayout, col: Column<'a, Message>) -> Element<'a, Message> {
    let body = col.width(Fill).max_width(layout.pane);
    scrollable(body)
        .direction(Direction::Vertical(
            Scrollbar::new()
                .width(SCROLLBAR_WIDTH)
                .scroller_width(SCROLLBAR_WIDTH),
        ))
        .width(Fill)
        .height(Fill)
        .into()
}

fn section_card(id: SettingsSection, app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    match id {
        SettingsSection::Daemon => daemon_card(app, demo),
        SettingsSection::Capture => capture_card(app, demo),
        SettingsSection::Companion => settings_card("Companion", companion_body(app, demo)),
        SettingsSection::Player => player_card(app, demo),
        SettingsSection::AutoDetect => settings_card("Auto-detect", auto_detect_body(app, demo)),
        SettingsSection::Sync => settings_card("Website sync", sync_body(app, demo)),
        SettingsSection::Ocr => ocr_card(app, demo),
        SettingsSection::Diagnostics => settings_card("Diagnostics", diagnostics_body(app, demo)),
        SettingsSection::Data => settings_card("Stored data", data_body(app, demo)),
    }
}

fn section_columns<'a>(columns: Vec<Vec<Element<'a, Message>>>) -> Element<'a, Message> {
    if columns.len() <= 1 {
        let mut col = column![].spacing(GRID_GAP).width(Fill);
        if let Some(cards) = columns.into_iter().next() {
            for card in cards {
                col = col.push(card);
            }
        }
        return col.into();
    }
    let mut row = Row::new()
        .spacing(GRID_GAP)
        .align_y(Alignment::Start)
        .width(Fill);
    for cards in columns {
        let mut col = column![].spacing(GRID_GAP).width(Fill);
        for card in cards {
            col = col.push(card);
        }
        row = row.push(col);
    }
    row.into()
}

fn daemon_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let body = column![service_body(app, demo), updates_body(app, demo)].spacing(6);
    settings_card("Tracker service", body.into())
}

/// Status, pid and the start, stop and login buttons.
pub(crate) fn service_body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let d = &app.daemon;
    let mut actions = row![].spacing(8).align_y(Alignment::Center);
    if !demo && !app.daemon_busy {
        if d.running() {
            actions = actions
                .push(action_btn(
                    "Stop",
                    false,
                    Message::Daemon(crate::daemon::DaemonVerb::Stop),
                ))
                .push(action_btn(
                    "Restart",
                    true,
                    Message::Daemon(crate::daemon::DaemonVerb::Restart),
                ));
        } else {
            actions = actions.push(action_btn(
                "Start",
                true,
                Message::Daemon(crate::daemon::DaemonVerb::Start),
            ));
        }
        if d.service_installed {
            actions = actions.push(action_btn(
                if d.autostart {
                    "Don’t start on login"
                } else {
                    "Start on login"
                },
                false,
                Message::ToggleAutostart,
            ));
        }
    }

    let mut body = column![
        row![
            text(d.status_label())
                .size(SIZE_BODY)
                .font(FONT_SEMIBOLD)
                .color(if d.running() { theme::OK } else { TEXT_2 }),
            space().width(Fill),
            text(if let Some(pid) = d.pid {
                format!("process {pid}")
            } else {
                String::new()
            })
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        ]
        .align_y(Alignment::Center),
        text(if d.service_installed {
            if d.autostart {
                "Starts with this computer"
            } else {
                "Installed, will not start on login"
            }
        } else {
            "No login service installed. Start launches the tracker directly"
        })
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
    ]
    .spacing(6);

    if app.daemon_busy {
        body = body.push(
            text("Working…")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }
    body = body.push(actions);
    body.into()
}

/// How this install updates, the copy button and the pre-release switch.
pub(crate) fn updates_body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut body = column![].spacing(6);
    let install = install_row(
        app.package_probe(),
        app.update.as_ref().map(|i| i.latest.as_str()),
        app.installed_version.as_deref(),
    );
    body = body.push(
        text(install.caption)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
    );
    body = body.push(
        text(install.line)
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(if install.copy { theme::ACCENT } else { TEXT_2 }),
    );
    if install.copy && !demo {
        body = body.push(action_btn("Copy command", false, Message::CopyUpdateCmd));
    }
    body = body.push(prerelease_updates_block(app));
    body.into()
}

fn capture_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let route = app.settings.capture_route;
    let session = app.session;
    let mut route_chips = row![].spacing(8);
    for choice in capture_route_chips(&app.desktop, session, route) {
        route_chips = route_chips.push(output_chip(
            capture_route_label(choice),
            route == choice,
            Message::SelectCaptureRoute(choice),
        ));
    }

    let mut chips = row![output_chip(
        "Auto (first monitor)",
        app.settings.capture_output.is_empty(),
        Message::SelectOutput(None),
    )]
    .spacing(8);
    for name in &app.outputs {
        chips = chips.push(output_chip(
            name,
            app.settings.capture_output == *name,
            Message::SelectOutput(Some(name.clone())),
        ));
    }

    let mut body = column![
        field_caption("Capture method"),
        route_chips.wrap(),
        text(capture_route_help(session))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(6);
    if capture_shared::shows_kde_direct_help(app) {
        body = body.push(
            text(KDE_DIRECT_HELP)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }
    body = capture_shared::push_route_notes(body, app);
    body = body.push(field_caption("Monitor")).push(chips.wrap()).push(
        text(format!(
            "Detected: {}",
            capture_shared::backend_label(app.backend)
        ))
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
    );

    if let Some(note) = app.capture_env_note() {
        body = body.push(
            text(note)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::DANGER),
        );
    }

    if !demo {
        let label = capture_shared::capture_button_label(app.backend.is_some(), app.capturing);
        let mut btn = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
            .padding(Padding::from([8, 16]))
            .style(theme::chip(true));
        if capture_shared::can_capture(app) {
            btn = btn.on_press(Message::CaptureNow);
        }
        body = body.push(btn);
    }

    if let Some(err) = capture_shared::preview_error(app) {
        body = body.push(err);
    }

    if let Some((handle, when)) = &app.preview {
        body = body.push(
            text(format!("Preview: {when}"))
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
        body = body.push(
            iced::widget::image(handle.clone())
                .width(Fill)
                .height(iced::Length::Fixed(280.0)),
        );
        body = body.push(
            text(capture_shared::PREVIEW_CAPTION)
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }

    settings_card("Capture", body.into())
}

pub(crate) fn companion_body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut body = column![
        checkbox(app.settings.overlay_hotkey_enabled)
            .label("Keyboard shortcut to hide or show the overlay")
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::OverlayHotkey, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
        hint("Works in fullscreen. Overlay is click-through."),
    ]
    .spacing(6);

    if app.settings.overlay_hotkey_enabled {
        body = body.push(field_input(
            "Shortcut",
            crate::hotkey::DEFAULT_BIND,
            &app.settings.overlay_hotkey,
            SettingsField::OverlayHotkey,
            false,
            demo,
            Some("Super = Windows key. Join with +."),
        ));
    }

    body.into()
}

/// In-game name and session window, side by side.
fn player_fields_row<'a>(
    player_name: &'a str,
    session_window_secs: &'a str,
    demo: bool,
) -> Element<'a, Message> {
    compact_row(
        Some(field_input(
            "In-game name",
            "e.g. FROZEN",
            player_name,
            SettingsField::PlayerName,
            false,
            demo,
            Some("Scoreboard name as shown in-game, without #1234."),
        )),
        Some(field_input(
            "Session window (seconds)",
            "1800",
            session_window_secs,
            SettingsField::SessionWindow,
            false,
            demo,
            Some("Nearby games count as one session."),
        )),
    )
}

fn player_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let body = column![
        player_fields_row(
            &app.settings.player_name,
            &app.settings.session_window_secs,
            demo,
        ),
        field_input(
            "Game process names",
            "Overwatch.exe",
            &app.settings.game_process_names,
            SettingsField::ProcessNames,
            false,
            demo,
            Some("Comma-separated. Empty = always allow capture."),
        ),
    ]
    .spacing(6);
    settings_card("Player and sessions", body.into())
}

pub(crate) fn auto_detect_body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut body = column![
        checkbox(app.settings.auto_detect_enabled)
            .label("Watch for match start and end")
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::AutoDetect, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
    ]
    .spacing(6);

    if app.settings.auto_detect_enabled {
        body = body.push(compact_row(
            Some(field_input(
                "Check every (seconds)",
                "auto (1.5 s with new reader)",
                &app.settings.poll_interval_secs,
                SettingsField::PollInterval,
                false,
                demo,
                None,
            )),
            Some(field_input(
                "Wait after detection (seconds)",
                "120",
                &app.settings.cooldown_secs,
                SettingsField::Cooldown,
                false,
                demo,
                None,
            )),
        ));
        body = body.push(hint(POLL_INTERVAL_HELP));
    }

    body.into()
}

pub(crate) fn sync_body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut body = column![field_input(
        "Website URL",
        "https://your-site.com",
        &app.settings.sync_url,
        SettingsField::SyncUrl,
        false,
        demo,
        None,
    ),]
    .spacing(6);
    if let Some(warning) = sync_url_problem(&app.settings.sync_url) {
        body = body.push(
            text(warning)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::DANGER),
        );
    }
    body = body.push(field_input(
        "Account token",
        "paste the token from the website",
        &app.settings.sync_token,
        SettingsField::SyncToken,
        true,
        demo,
        Some("Both needed to upload. Blank either to turn sync off."),
    ));
    body.into()
}

fn ocr_card(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let blocks = ocr_card_blocks(app.ocr_switch.daemon_off);
    let mut body = column![classic_ocr_block(app)].spacing(6);
    if blocks.reading_model {
        body = body.push(reading_model_block(app, demo));
    }
    if blocks.new_reader {
        body = body.push(new_reader_block(app));
    }
    body = body.push(shadow_reader_block(app));
    settings_card("Scoreboard reading", body.into())
}

fn classic_ocr_block(app: &TrackerApp) -> Element<'_, Message> {
    let mut body = column![
        checkbox(app.settings.classic_ocr)
            .label(CLASSIC_OCR_LABEL)
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::ClassicOcr, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
    ]
    .spacing(4);
    let lines = ocr_hint_lines(&OcrRead::needing_ocr());
    body = body.push(hint(lines.hint));
    if lines.restart_and_packs {
        body = body
            .push(hint(CLASSIC_OCR_RESTART))
            .push(hint(pack_status_line(app.ocr_switch.packs.as_ref())));
    }
    for line in ocr_problem_lines(app, |problem| {
        text(problem.to_string())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(theme::WARN)
            .into()
    }) {
        body = body.push(line);
    }
    body.into()
}

/// READ-1: the model is not needed to read the scoreboard under the default
/// new reader. Without it, classic Tesseract reads fall back to `eng`.
pub(crate) fn reading_model_status(installed: bool) -> &'static str {
    if installed {
        "Reading model is installed"
    } else {
        "Reading model is missing. Classic OCR reads use the generic English model, which is less accurate."
    }
}

pub(crate) const READING_MODEL_HINT: &str = "\
Install puts back the game-font model that ships with the tracker. It does not \
train one on this computer.";

pub(crate) fn reading_model_block(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let installed = reading_model_status(app.tessdata_installed);
    let mut actions = row![].spacing(8);
    if !demo && !app.tessdata_busy {
        actions = actions.push(action_btn(
            "Install reading model",
            true,
            Message::InstallModel,
        ));
    }
    let mut body = column![
        text(installed)
            .size(SIZE_BODY)
            .font(FONT_MEDIUM)
            .color(if app.tessdata_installed {
                theme::OK
            } else {
                theme::WARN
            }),
        hint(READING_MODEL_HINT),
    ]
    .spacing(6);
    if app.tessdata_busy {
        body = body.push(
            text("Working on the reading model…")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }
    body = body.push(actions);
    body.into()
}

fn prerelease_updates_block(app: &TrackerApp) -> Element<'_, Message> {
    column![
        checkbox(app.settings.prerelease_updates)
            .label(PRERELEASE_UPDATES_LABEL)
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::PrereleaseUpdates, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
        hint(PRERELEASE_UPDATES_HINT),
        hint(PRERELEASE_UPDATES_APPLIES),
    ]
    .spacing(4)
    .into()
}

fn new_reader_block(app: &TrackerApp) -> Element<'_, Message> {
    column![
        checkbox(app.settings.new_reader)
            .label(NEW_READER_LABEL)
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::NewReader, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
        hint(NEW_READER_HINT),
        hint(NEW_READER_RESTART),
    ]
    .spacing(4)
    .into()
}

pub(crate) fn shadow_reader_block(app: &TrackerApp) -> Element<'_, Message> {
    let mut toggle = checkbox(app.settings.displayed_reader().file_flag())
        .label(SHADOW_READER_LABEL)
        .size(16)
        .text_size(SIZE_BODY)
        .style(checkbox_style);
    if shadow_toggle_interactive(app.settings.shadow.locked) {
        toggle = toggle.on_toggle(|v| Message::SettingsToggle(SettingsToggle::ShadowRecognizer, v));
    }
    let mut body = column![
        toggle,
        hint(SHADOW_READER_HINT),
        hint(SHADOW_READER_RESTART),
    ]
    .spacing(4);
    if app.settings.shadow.locked {
        body = body.push(hint(SHADOW_READER_ENV_NOTE));
    }
    body.into()
}

/// False when the env override is on. The checkbox stays checked and does not
/// emit toggles, so Save cannot persist the variable.
pub(crate) fn shadow_toggle_interactive(locked: bool) -> bool {
    !locked
}

/// Help under the poll interval field. Empty or 4 is auto, which polls every
/// 1.5 s with the new reader and every 4 s with classic OCR.
pub(crate) const POLL_INTERVAL_HELP: &str = "Empty means auto: 1.5 s with the new reader, 4 s with classic OCR. 4 also means auto. For slower checks on a low-power laptop, enter 5 or more.";
/// Help under "Save debug images from each capture". The accepted and
/// rejected scoreboard frames are not blanked, so it says whose names they
/// show and where they stay.
pub(crate) const DEBUG_IMAGES_HELP: &str = "Writes Tab crops, the accepted and rejected scoreboard frames, and poll Victory/Defeat evidence frames (not every tick). The scoreboard frames show other players' names. These images stay on this PC only. Slows capture, so leave it off unless diagnosing.";

pub(crate) fn diagnostics_body(app: &TrackerApp, _demo: bool) -> Element<'_, Message> {
    let body = column![
        checkbox(app.settings.debug_ocr)
            .label("Save debug images from each capture")
            .on_toggle(|v| Message::SettingsToggle(SettingsToggle::DebugOcr, v))
            .size(16)
            .text_size(SIZE_BODY)
            .style(checkbox_style),
        hint(DEBUG_IMAGES_HELP),
    ]
    .spacing(6);
    body.into()
}

pub(crate) fn data_body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let running = app.daemon.running();
    let mut copy = column![hint(
        "Compact rewrites the store and keeps one backup. Delete removes local matches and the saved screen-share permission, not the website. The next screen share asks for permission again."
    ),]
    .spacing(6);

    if running {
        copy = copy.push(
            text("Stop the tracker before compact or delete.")
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(theme::WARN),
        );
    }
    if app.confirm_clear {
        copy = copy.push(
            text("Really delete all local match history? The screen-share permission is forgotten too. This cannot be undone.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::DANGER),
        );
    }
    if app.vacuum_busy {
        copy = copy.push(
            text("Compacting…")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }

    let show_actions = !demo && !running && !app.vacuum_busy;
    let mut actions = row![].spacing(8).align_y(Alignment::Center);
    if show_actions {
        actions = actions.push(action_btn("Compact stored data", false, Message::Vacuum));
        if app.confirm_clear {
            actions = actions
                .push(action_btn("Cancel", false, Message::ConfirmClear))
                .push(
                    button(
                        text("Yes, delete everything")
                            .size(SIZE_META)
                            .font(FONT_SEMIBOLD)
                            .color(TEXT),
                    )
                    .padding(Padding::from([8, 16]))
                    .style(theme::danger_btn(true))
                    .on_press(Message::ClearData),
                );
        } else {
            actions = actions.push(
                button(
                    text("Delete all local match data")
                        .size(SIZE_META)
                        .font(FONT_SEMIBOLD)
                        .color(theme::DANGER),
                )
                .padding(Padding::from([8, 16]))
                .style(theme::danger_btn(false))
                .on_press(Message::ConfirmClear),
            );
        }
    }

    // Buttons under the copy, like the other cards. Side by side, this card
    // spans the pane, so the buttons sat up to 3000 px from the text on a
    // wide window and squeezed the text to a sliver on a narrow one.
    let mut body = column![copy.width(Fill)].spacing(8).width(Fill);
    if show_actions {
        body = body.push(actions.wrap());
    }
    body.into()
}

/// A "text … button" row in a card that spans the pane: left-aligned and no
/// wider than [`ACTION_ROW_MAX`], so the button stays near its label.
fn action_row<'a>(row: Row<'a, Message>) -> Element<'a, Message> {
    container(row.width(Fill))
        .width(Fill)
        .max_width(ACTION_ROW_MAX)
        .into()
}

fn report_row() -> Element<'static, Message> {
    settings_card(
        "Report a problem",
        action_row(
            row![
                text(REPORT_HINT)
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3)
                    .width(Fill),
                button(
                    text("Report a problem")
                        .size(SIZE_META)
                        .font(FONT_SEMIBOLD)
                        .color(TEXT),
                )
                .padding(Padding::from([8, 16]))
                .style(theme::ghost_btn())
                .on_press(Message::Report(crate::report::ReportMessage::Open(None))),
            ]
            .align_y(Alignment::Center)
            .spacing(GRID_GAP),
        ),
    )
}

fn uninstall_row() -> Element<'static, Message> {
    settings_card(
        "Uninstall",
        action_row(
            row![
                text("Removes the tracker from this computer.")
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3)
                    .width(Fill),
                uninstall_button(Some(Message::OpenUninstall), Padding::from([8, 16])),
            ]
            .align_y(Alignment::Center)
            .spacing(GRID_GAP),
        ),
    )
}

fn save_footer(demo: bool) -> Element<'static, Message> {
    if demo {
        return space().height(0).into();
    }
    container(action_row(
        row![
            text("Writes config and the companion shortcut.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
            space().width(Fill),
            action_btn(SAVE_LABEL, true, Message::SaveSettings),
        ]
        .align_y(Alignment::Center)
        .spacing(GRID_GAP),
    ))
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

fn settings_card<'a>(title: &'static str, body: Element<'a, Message>) -> Element<'a, Message> {
    container(
        column![
            text(title).size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),
            body,
        ]
        .spacing(8),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

/// One or two compact fields. Each field's column is its share of the row
/// ([`field_fill_portion`]); the input inside keeps its own cap, so it does
/// not stretch across an ultrawide card. There is no trailing spacer: it
/// took a third of the row and squeezed both fields (and wrapped their
/// hints) on a narrow card.
fn compact_row<'a>(
    left: Option<Element<'a, Message>>,
    right: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut r = row![]
        .spacing(GRID_GAP)
        .align_y(Alignment::Start)
        .width(Fill);
    if let Some(left) = left {
        r = r.push(left);
    }
    if let Some(right) = right {
        r = r.push(right);
    }
    r.into()
}

fn field_input<'a>(
    label: &'static str,
    placeholder: &'static str,
    value: &'a str,
    field: SettingsField,
    secret: bool,
    demo: bool,
    hint_copy: Option<&'static str>,
) -> Element<'a, Message> {
    let mut input = text_input(placeholder, value)
        .padding(Padding::from([8, 10]))
        .size(SIZE_BODY)
        .style(theme::text_input_style)
        .secure(secret)
        .width(Fill);
    if !demo {
        input = input.on_input(move |v| Message::SettingsText(field, v));
    }
    let input = container(input)
        .width(Fill)
        .max_width(field_max_width(field));
    let mut col = column![field_caption(label), input].spacing(4);
    if let Some(copy) = hint_copy {
        col = col.push(hint(copy));
    }
    // No max width on the column: a row ignores it (see `field_fill_portion`).
    // The input is capped above and the hint by `HELP_MEASURE`.
    col.width(Length::FillPortion(field_fill_portion(field)))
        .into()
}

#[cfg(test)]
mod tests {
    /// READ-1: the default new reader does not need the model, so a missing
    /// model must not say the scoreboard cannot be read.
    #[test]
    fn a_missing_reading_model_does_not_claim_the_scoreboard_is_unreadable() {
        let missing = super::reading_model_status(false);
        assert!(!missing.contains("to read the scoreboard"), "{missing}");
        assert!(missing.contains("less accurate"), "{missing}");
        assert!(missing.ends_with('.'), "{missing}");
        assert_eq!(
            super::reading_model_status(true),
            "Reading model is installed"
        );
        assert!(!super::READING_MODEL_HINT.contains("retrain"));
    }

    #[test]
    fn debug_images_help_names_the_scoreboard_frames_and_where_they_stay() {
        // R2 on #244: the setting also saves unblanked scoreboard frames.
        for part in [
            "accepted and rejected scoreboard frames",
            "other players' names",
            "stay on this PC only",
        ] {
            assert!(super::DEBUG_IMAGES_HELP.contains(part), "{part}");
        }
    }

    use super::*;
    use std::path::PathBuf;

    fn base() -> Config {
        Config {
            data_dir: PathBuf::from("/tmp/sst-ui-settings-test"),
            ocr_threads: Some(2),
            ..Config::default()
        }
    }

    #[test]
    fn empty_optional_fields_become_none() {
        let form = SettingsForm {
            capture_output: "  ".into(),
            capture_route: CaptureRoute::Auto,
            player_name: String::new(),
            session_window_secs: "1800".into(),
            game_process_names: String::new(),
            auto_detect_enabled: true,
            poll_interval_secs: String::new(),
            cooldown_secs: "120".into(),
            sync_url: String::new(),
            sync_token: "tok".into(),
            debug_ocr: false,
            overlay_hotkey: crate::hotkey::DEFAULT_BIND.into(),
            overlay_hotkey_enabled: true,
            shadow: Config::shadow_control(false, None),
            new_reader: false,
            classic_ocr: true,
            prerelease_updates: false,
            prerelease_updates_shown: false,
        };
        let c = form.to_config(&base());
        assert!(c.capture_output.is_none());
        assert!(c.player_name.is_none());
        assert!(c.sync.is_none());
        assert!(c.game_process_names.is_empty());
        assert!(!c.shadow_recognizer);
        assert_eq!(c.data_dir, PathBuf::from("/tmp/sst-ui-settings-test"));
        assert_eq!(c.ocr_threads, Some(2));
    }

    #[test]
    fn sync_requires_both_fields() {
        assert!(sync_from_fields("", "tok").is_none());
        assert!(sync_from_fields("https://x", "").is_none());
        let s = sync_from_fields(" https://x ", " tok ").unwrap();
        assert_eq!(s.server_url, "https://x");
        assert_eq!(s.token, "tok");
    }

    #[test]
    fn sync_url_scheme_blocks_cleartext_and_allows_loopback() {
        assert!(sync_url_problem("https://crew.example").is_none());
        assert!(sync_url_problem("http://localhost:3030").is_none());
        assert!(sync_url_problem("http://127.0.0.1").is_none());
        assert!(sync_url_problem("http://[::1]:9").is_none());
        assert!(sync_url_problem("").is_none());
        assert!(sync_url_problem("   ").is_none());
        let warn = sync_url_problem("http://crew.example").expect("cleartext warning");
        assert!(warn.contains("https"), "{warn}");
        assert!(sync_from_fields("http://crew.example", "tok").is_none());
        assert!(sync_from_fields("http://127.0.0.1:3030", "tok").is_some());
        assert!(sync_from_fields("https://crew.example", "tok").is_some());

        let mut form = SettingsForm::from_config(&base());
        form.sync_url = "http://example.com".into();
        form.sync_token = "secret".into();
        assert!(
            form.to_config(&base()).sync.is_none(),
            "an unsafe URL must not become a saved sync config"
        );
    }

    #[test]
    fn number_fallbacks_match_config_defaults() {
        assert_eq!(parse_u64("nope", 1800), 1800);
        assert_eq!(parse_u64(" 90 ", 4), 90);
        let mut form = SettingsForm::from_config(&Config::default());
        form.poll_interval_secs = "x".into();
        form.cooldown_secs = "".into();
        form.session_window_secs = "oops".into();
        let c = form.to_config(&base());
        assert_eq!(c.auto_detect.poll_interval_secs, None);
        assert_eq!(c.auto_detect.cooldown_secs, 120);
        assert_eq!(c.session_window_secs, 1800);
    }

    #[test]
    fn roundtrip_preserves_today_fields() {
        let mut base = base();
        base.capture_output = Some("DP-1".into());
        base.player_name = Some("Ada".into());
        base.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "secret".into(),
        });
        base.auto_detect = AutoDetectConfig {
            enabled: false,
            poll_interval_secs: Some(8),
            poll_interval_ms: Some(750),
            cooldown_secs: 60,
        };
        base.screencast_max_fps = Some(4);
        base.session_window_secs = 900;
        base.finished_game_close_secs = 240;
        base.game_process_names = vec!["Overwatch.exe".into()];
        base.debug_ocr = true;
        base.shadow_recognizer = true;
        let form = SettingsForm::from_config_and_shadow(
            &base,
            Config::shadow_control(base.shadow_recognizer, None),
        );
        let out = form.to_config(&base);
        assert_eq!(form.persisted_reader(), ScoreboardReader::Extra);
        assert_eq!(out.capture_output.as_deref(), Some("DP-1"));
        assert_eq!(out.player_name.as_deref(), Some("Ada"));
        assert_eq!(
            out.sync.as_ref().map(|s| s.server_url.as_str()),
            Some("https://crew.example")
        );
        assert_eq!(out.auto_detect.poll_interval_secs, Some(8));
        assert_eq!(out.auto_detect.poll_interval_ms, Some(750));
        assert_eq!(out.screencast_max_fps, Some(4));
        assert_eq!(out.session_window_secs, 900);
        assert_eq!(out.finished_game_close_secs, 240);
        assert_eq!(out.game_process_names, vec!["Overwatch.exe"]);
        assert!(out.debug_ocr);
        assert!(out.shadow_recognizer);
        assert_eq!(out.ocr_threads, Some(2));
        assert_eq!(form.overlay_hotkey, crate::hotkey::DEFAULT_BIND);
        assert!(form.overlay_hotkey_enabled);
    }

    #[test]
    fn short_numeric_fields_do_not_span_ultrawide() {
        for field in [
            SettingsField::SessionWindow,
            SettingsField::PollInterval,
            SettingsField::Cooldown,
        ] {
            assert!(
                field_max_width(field) <= FIELD_NUMERIC,
                "{field:?} must stay a compact numeric"
            );
        }
        assert_eq!(field_max_width(SettingsField::SessionWindow), FIELD_NUMERIC);
        assert_eq!(field_max_width(SettingsField::PollInterval), FIELD_NUMERIC);
        assert_eq!(field_max_width(SettingsField::Cooldown), FIELD_NUMERIC);
    }

    #[test]
    fn settings_cards_use_maps_density_tokens() {
        use crate::theme::{PAGE_PAD_X, PAGE_PAD_Y};
        assert_eq!(PAGE_PAD_Y, 24.0);
        assert_eq!(PAGE_PAD_X, 32.0);
        assert_eq!(PAD_INNER, 12.0);
        assert_eq!(theme::RADIUS_CARD, 16.0);
        assert_eq!(theme::GRID_GAP, 12.0);
        assert_eq!(theme::SIZE_LABEL, 11.0);
        assert_eq!(theme::ACCENT, {
            iced::Color::from_rgb(
                0x8f as f32 / 255.0,
                0x73 as f32 / 255.0,
                0xff as f32 / 255.0,
            )
        });
    }

    /// G4: was `settings_section_grid_is_one_or_two_columns`. Wide windows
    /// now get three and four columns (see `layout` tests for the table).
    #[test]
    fn settings_section_grid_is_one_to_four_columns() {
        use crate::layout::{MAX_SETTINGS_COLUMNS, settings_columns};
        assert_eq!(settings_columns(200.0), 1);
        assert_eq!(settings_columns(480.0), 1);
        assert_eq!(settings_columns(972.0), 2);
        assert_eq!(settings_columns(1688.0), 3);
        assert_eq!(settings_columns(3208.0), 4);
        // The literal: five columns would leave some with one card.
        assert_eq!(settings_columns(10_000.0), 4);
        assert!(MAX_SETTINGS_COLUMNS * 2 <= MASONRY_SECTIONS.len());
    }

    // ---- Headless layout -------------------------------------------------
    //
    // Iced's own layout pass on the real view pieces, with the software
    // renderer and the app fonts. No window is opened. None of these build a
    // `TrackerApp`, so no tray icon is made and no companion is started.

    use iced::advanced::layout::{Limits, Node};
    use iced::advanced::widget::Tree;
    use iced::widget::column;

    fn headless() -> iced::Renderer {
        use iced::advanced::renderer::Headless;
        let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
            FONT_MEDIUM,
            iced::Pixels(SIZE_BODY),
            Some("tiny-skia"),
        ))
        .expect("the tiny-skia renderer needs no display");
        for bytes in [
            theme::FONT_BYTES_MEDIUM,
            theme::FONT_BYTES_SEMIBOLD,
            theme::FONT_BYTES_BOLD,
            theme::FONT_BYTES_EXTRABOLD,
        ] {
            iced::advanced::graphics::text::font_system()
                .write()
                .expect("font system lock")
                .load_font(std::borrow::Cow::Borrowed(bytes));
        }
        renderer
    }

    /// Lay `element` out in a `width` x 900 px box.
    fn lay_out(mut element: Element<'_, Message>, width: f32) -> Node {
        let renderer = headless();
        let mut tree = Tree::new(&element);
        element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &Limits::new(iced::Size::ZERO, iced::Size::new(width, 900.0)),
        )
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.5
    }

    fn tall_card<'a>() -> Element<'a, Message> {
        container(space().height(700)).width(Fill).into()
    }

    /// The visual finding on #258: the scrollbar was drawn over the last
    /// 10 px of the right-hand cards whenever the page scrolled.
    #[test]
    fn cards_stop_a_grid_gap_short_of_the_scrollbar() {
        use crate::layout::{cell_width, content_width_for_window};
        for window in [1280.0, 1600.0, 1920.0, 2560.0, 3440.0] {
            let content = content_width_for_window(window);
            let layout = settings_layout(content);
            assert!(layout.columns >= 2, "window {window}");
            let cards = section_columns(
                (0..layout.columns)
                    .map(|_| vec![tall_card(), tall_card()])
                    .collect(),
            );
            let col = column![cards, tall_card()].spacing(GRID_GAP).width(Fill);
            let frame = lay_out(scroll_frame(layout, col), content);

            // The scrollable fills the content width; its bar is the last
            // SCROLLBAR_WIDTH px of that.
            assert!(near(frame.size().width, content), "window {window}");
            let bar_left = content - SCROLLBAR_WIDTH;
            let pane = &frame.children()[0];
            assert!(
                pane.size().height > frame.size().height,
                "window {window}: the page must scroll for this test"
            );
            let clear = bar_left - GRID_GAP + 0.01;
            let pane_x = pane.bounds().x;
            assert!(near(pane.bounds().width, layout.pane), "window {window}");
            assert!(pane_x + pane.bounds().width <= clear, "window {window}");

            let row = &pane.children()[0];
            assert_eq!(row.children().len(), layout.columns, "window {window}");
            let card = cell_width(layout.pane, layout.columns, GRID_GAP);
            for column in row.children() {
                let right = pane_x + row.bounds().x + column.bounds().x + column.bounds().width;
                assert!(right <= clear, "window {window}: card ends at {right}");
                assert!(near(column.bounds().width, card), "window {window}");
            }
            let strip = &pane.children()[1];
            let right = pane_x + strip.bounds().x + strip.bounds().width;
            assert!(right <= clear, "window {window}: strip ends at {right}");
        }
    }

    /// Card inner width on a window: the card less its padding.
    fn card_inner(window: f32) -> f32 {
        use crate::layout::{cell_width, content_width_for_window};
        let layout = settings_layout(content_width_for_window(window));
        cell_width(layout.pane, layout.columns, GRID_GAP) - 2.0 * PAD_INNER
    }

    /// One line of `SIZE_LABEL` help copy is about 14 px tall, two are 29.
    const ONE_HINT_LINE: f32 = 20.0;

    #[test]
    fn name_and_session_fields_share_the_row_four_to_three() {
        // 1280 is the default window. An even split wrapped the name hint to
        // a second line holding only "#1234."; a third spacer child (the old
        // row) squeezed both fields to a third.
        let inner = card_inner(1280.0);
        let row = lay_out(player_fields_row("", "1800", true), inner);
        assert_eq!(row.children().len(), 2, "no trailing spacer");
        let (name, session) = (&row.children()[0], &row.children()[1]);
        let free = inner - GRID_GAP;
        assert!(near(name.bounds().width, free * 4.0 / 7.0), "{name:?}");
        assert!(
            near(session.bounds().width, free * 3.0 / 7.0),
            "{session:?}"
        );
        // Field column: caption, input, hint.
        for (label, field) in [("name", name), ("session", session)] {
            assert_eq!(field.children().len(), 3, "{label}");
            let hint = field.children()[2].bounds();
            assert!(
                hint.width > 100.0,
                "{label} hint was not measured: {hint:?}"
            );
            assert!(
                hint.height < ONE_HINT_LINE,
                "{label} hint wraps at 1280: {hint:?}"
            );
        }
    }

    #[test]
    fn inputs_keep_their_caps_when_the_card_is_wide() {
        // 2560: a 761 px card. Both field columns are wider than their
        // inputs; the inputs must not follow them.
        let inner = card_inner(2560.0);
        let row = lay_out(player_fields_row("", "1800", true), inner);
        let (name, session) = (&row.children()[0], &row.children()[1]);
        assert!(name.bounds().width > FIELD_SHORT + 50.0, "{name:?}");
        assert!(near(name.children()[1].bounds().width, FIELD_SHORT));
        assert!(near(session.children()[1].bounds().width, FIELD_NUMERIC));

        // A field on its own in a card: the input stops at its cap too.
        let alone = lay_out(
            field_input(
                "Website URL",
                "https://your-site.com",
                "",
                SettingsField::SyncUrl,
                false,
                true,
                None,
            ),
            inner,
        );
        assert!(near(alone.bounds().width, inner));
        assert!(near(alone.children()[1].bounds().width, FIELD_TEXT));

        // Narrower than its cap, an input takes what the column has.
        let narrow = lay_out(player_fields_row("", "1800", true), card_inner(1280.0));
        let name = &narrow.children()[0];
        assert!(name.bounds().width < FIELD_SHORT);
        assert!(near(name.children()[1].bounds().width, name.bounds().width));
    }

    #[test]
    fn help_copy_follows_the_card_up_to_the_measure() {
        let wide = lay_out(
            column![hint(DEBUG_IMAGES_HELP)].width(Fill).into(),
            card_inner(2560.0),
        );
        let w = wide.children()[0].bounds().width;
        assert!(w <= HELP_MEASURE + 0.01, "hint is {w} px wide");
        assert!(
            w > 520.0,
            "hint is {w} px wide; the old 520 px wrap is back"
        );

        let inner = card_inner(1280.0);
        let narrow = lay_out(column![hint(DEBUG_IMAGES_HELP)].width(Fill).into(), inner);
        let w = narrow.children()[0].bounds().width;
        assert!(w <= inner + 0.01, "hint is {w} px in a {inner} px card");
        assert!(w > 300.0, "hint is {w} px wide");
    }

    #[test]
    fn a_full_width_row_keeps_its_button_near_the_text() {
        // Report a problem spans the pane. At 2560 the button was 2200 px
        // from the label.
        let pane = settings_layout(crate::layout::content_width_for_window(2560.0)).pane;
        let card = lay_out(report_row(), pane);
        assert!(near(card.bounds().width, pane), "the card still spans");
        // Card: column [title, action row]. Action row: [text, button].
        let action = &card.children()[0].children()[1];
        assert!(near(action.bounds().width, ACTION_ROW_MAX), "{action:?}");
        let row = &action.children()[0];
        let button = row.children()[1].bounds();
        assert!(
            button.x + button.width <= ACTION_ROW_MAX + 0.5,
            "{button:?}"
        );

        // One column (720 minimum window): unchanged, the row fills the card.
        let pane = settings_layout(crate::layout::content_width_for_window(720.0)).pane;
        let card = lay_out(uninstall_row(), pane);
        let action = &card.children()[0].children()[1];
        assert!(near(action.bounds().width, pane - 2.0 * PAD_INNER));
    }

    #[test]
    fn text_fields_are_capped_but_wider_than_numerics() {
        assert_eq!(field_max_width(SettingsField::SyncUrl), FIELD_TEXT);
        assert_eq!(field_max_width(SettingsField::SyncToken), FIELD_TEXT);
        assert_eq!(field_max_width(SettingsField::ProcessNames), FIELD_TEXT);
        assert_eq!(field_max_width(SettingsField::PlayerName), FIELD_SHORT);
        assert_eq!(field_max_width(SettingsField::OverlayHotkey), FIELD_SHORT);
    }

    #[test]
    fn masonry_omits_stored_data_and_keeps_every_other_section() {
        assert_eq!(MASONRY_SECTIONS.len(), 8);
        assert_eq!(SPANNING_SECTION, SettingsSection::Data);
        assert!(!MASONRY_SECTIONS.contains(&SettingsSection::Data));
        let all = [
            SettingsSection::Daemon,
            SettingsSection::Capture,
            SettingsSection::Companion,
            SettingsSection::Player,
            SettingsSection::AutoDetect,
            SettingsSection::Sync,
            SettingsSection::Ocr,
            SettingsSection::Diagnostics,
            SettingsSection::Data,
        ];
        for section in all {
            assert!(
                MASONRY_SECTIONS.contains(&section) || section == SPANNING_SECTION,
                "missing {section:?}"
            );
        }
    }

    #[test]
    fn two_column_pack_balances_and_leaves_no_empty_column() {
        let packed = pack_columns(MASONRY_SECTIONS, 2);
        assert_eq!(packed.len(), 2);
        assert_eq!(
            packed[0],
            [
                SettingsSection::Daemon,
                SettingsSection::Companion,
                SettingsSection::AutoDetect,
                SettingsSection::Ocr,
            ]
        );
        assert_eq!(
            packed[1],
            [
                SettingsSection::Capture,
                SettingsSection::Player,
                SettingsSection::Sync,
                SettingsSection::Diagnostics,
            ]
        );
        let left: u16 = packed[0].iter().copied().map(section_weight).sum();
        let right: u16 = packed[1].iter().copied().map(section_weight).sum();
        assert_eq!(left, right);
        assert!(!packed[0].contains(&SettingsSection::Data));
        assert!(!packed[1].contains(&SettingsSection::Data));
    }

    #[test]
    fn one_column_pack_keeps_input_order() {
        let packed = pack_columns(MASONRY_SECTIONS, 1);
        assert_eq!(packed, vec![MASONRY_SECTIONS.to_vec()]);
    }

    fn heights(packed: &[Vec<SettingsSection>]) -> Vec<u16> {
        packed
            .iter()
            .map(|c| c.iter().copied().map(section_weight).sum())
            .collect()
    }

    #[test]
    fn three_column_pack_is_as_even_as_the_weights_allow() {
        // 24 units over 3 columns: Player (4) has no partner summing to 4,
        // so 8/8/8 is impossible and 7/9/8 is the best split.
        let packed = pack_columns(MASONRY_SECTIONS, 3);
        assert_eq!(
            packed,
            vec![
                vec![SettingsSection::Daemon, SettingsSection::Player],
                vec![
                    SettingsSection::Capture,
                    SettingsSection::AutoDetect,
                    SettingsSection::Ocr,
                ],
                vec![
                    SettingsSection::Companion,
                    SettingsSection::Sync,
                    SettingsSection::Diagnostics,
                ],
            ]
        );
        assert_eq!(heights(&packed), vec![7, 9, 8]);
    }

    #[test]
    fn four_column_pack_is_two_cards_each_and_level() {
        let packed = pack_columns(MASONRY_SECTIONS, 4);
        assert_eq!(packed.len(), 4);
        for col in &packed {
            assert_eq!(col.len(), 2, "{packed:?}");
        }
        assert_eq!(heights(&packed), vec![6, 6, 6, 6]);
    }

    #[test]
    fn pack_balances_any_column_count_and_never_leaves_a_column_empty() {
        let max_weight = MASONRY_SECTIONS
            .iter()
            .copied()
            .map(section_weight)
            .max()
            .unwrap();
        for cols in 1..=12 {
            let packed = pack_columns(MASONRY_SECTIONS, cols);
            assert_eq!(
                packed.len(),
                cols.min(MASONRY_SECTIONS.len()),
                "column count for {cols}"
            );
            assert!(packed.iter().all(|c| !c.is_empty()), "{cols}: {packed:?}");
            // Every section exactly once, in input order within a column.
            let mut seen: Vec<SettingsSection> = packed.iter().flatten().copied().collect();
            assert_eq!(seen.len(), MASONRY_SECTIONS.len());
            for col in &packed {
                let idx: Vec<usize> = col
                    .iter()
                    .map(|s| MASONRY_SECTIONS.iter().position(|m| m == s).unwrap())
                    .collect();
                assert!(idx.windows(2).all(|w| w[0] < w[1]), "{cols}: {col:?}");
            }
            seen.sort_by_key(|s| MASONRY_SECTIONS.iter().position(|m| m == s));
            assert_eq!(seen, MASONRY_SECTIONS.to_vec());
            // Greedy shortest-column bound: no column is taller than the
            // shortest by more than one card.
            let h = heights(&packed);
            let spread = h.iter().max().unwrap() - h.iter().min().unwrap();
            assert!(spread <= max_weight, "{cols}: heights {h:?}");
        }
    }

    #[test]
    fn pack_columns_for_every_settings_width_fills_each_column() {
        use crate::layout::content_width_for_window;
        for window in [
            720.0, 1080.0, 1280.0, 1600.0, 1920.0, 2560.0, 3440.0, 5120.0,
        ] {
            let cols = settings_layout(content_width_for_window(window)).columns;
            let packed = pack_columns(MASONRY_SECTIONS, cols);
            assert_eq!(packed.len(), cols, "window {window}");
            assert!(packed.iter().all(|c| !c.is_empty()), "window {window}");
        }
    }

    #[test]
    fn save_footer_label_stays_primary_action() {
        assert_eq!(SAVE_LABEL, "Save settings");
        assert_eq!(theme::SIZE_LABEL, 11.0);
    }

    #[test]
    fn shadow_copy_is_plain() {
        for copy in [
            SHADOW_READER_LABEL,
            SHADOW_READER_HINT,
            SHADOW_READER_RESTART,
            SHADOW_READER_ENV_NOTE,
        ] {
            assert!(!copy.contains('—'), "{copy}");
            assert!(!copy.contains('–'), "{copy}");
        }
        assert_eq!(SHADOW_READER_LABEL, "Extra number reader (test)");
        assert!(SHADOW_READER_HINT.contains("private log"));
        assert!(SHADOW_READER_HINT.contains("saved games"));
        assert!(SHADOW_READER_HINT.contains("off by default"));
        assert!(SHADOW_READER_RESTART.contains("Restart"));
        assert!(SHADOW_READER_RESTART.contains("when it starts"));
        assert!(SHADOW_READER_ENV_NOTE.contains("SCUFFED_SHADOW_RECOGNIZER"));
        assert!(SHADOW_READER_ENV_NOTE.contains("this app"));
        assert!(SHADOW_READER_ENV_NOTE.contains("tracker service"));
        assert!(SHADOW_READER_ENV_NOTE.contains("not written"));
    }

    #[test]
    fn reader_choices_are_ocr_v1_then_the_extra_test_reader() {
        assert_eq!(
            ScoreboardReader::choices(),
            [ScoreboardReader::OcrV1, ScoreboardReader::Extra]
        );
        assert_eq!(ScoreboardReader::OcrV1.id(), "ocr-v1");
        assert_eq!(
            ScoreboardReader::Extra.id(),
            stat_tracker::shadow::digits::RECOGNIZER_ID
        );
        assert!(!ScoreboardReader::OcrV1.file_flag());
        assert!(ScoreboardReader::Extra.file_flag());
        assert!(shadow_toggle_interactive(false));
        assert!(!shadow_toggle_interactive(true));
    }

    #[test]
    fn shadow_toggle_round_trip_preserves_the_rest_of_the_file() {
        let raw = r#"
data_dir = "/tmp/sst-ui-settings-test"
player_name = "FrozenTear"
session_window_secs = 900
finished_game_close_secs = 240
debug_ocr = true
ocr_threads = 2
game_process_names = ["Overwatch.exe"]

[auto_detect]
enabled = false
poll_interval_secs = 8
cooldown_secs = 60
"#;
        let base: Config = toml::from_str(raw).unwrap();
        assert!(!base.shadow_recognizer);
        let mut form = SettingsForm::from_config_and_shadow(
            &base,
            Config::shadow_control(base.shadow_recognizer, None),
        );
        assert_eq!(form.displayed_reader(), ScoreboardReader::OcrV1);
        assert_eq!(form.persisted_reader(), ScoreboardReader::OcrV1);
        assert!(shadow_toggle_interactive(form.shadow.locked));

        form.set_toggle(SettingsToggle::ShadowRecognizer, true);
        assert_eq!(form.persisted_reader(), ScoreboardReader::Extra);
        assert_eq!(
            form.displayed_reader().id(),
            stat_tracker::shadow::digits::RECOGNIZER_ID
        );
        let saved = toml::to_string_pretty(&form.to_config(&base)).unwrap();
        assert!(saved.contains("shadow_recognizer = true"), "{saved}");
        assert!(saved.contains("FrozenTear"), "{saved}");
        assert!(saved.contains("finished_game_close_secs = 240"), "{saved}");
        assert!(saved.contains("ocr_threads = 2"), "{saved}");
        assert!(saved.contains("session_window_secs = 900"), "{saved}");

        let loaded: Config = toml::from_str(&saved).unwrap();
        let mut again = SettingsForm::from_config_and_shadow(
            &loaded,
            Config::shadow_control(loaded.shadow_recognizer, None),
        );
        let back = again.to_config(&loaded);
        assert!(back.shadow_recognizer);
        assert_eq!(back.player_name.as_deref(), Some("FrozenTear"));
        assert_eq!(back.finished_game_close_secs, 240);
        assert_eq!(back.ocr_threads, Some(2));
        assert_eq!(back.session_window_secs, 900);
        assert!(back.debug_ocr);
        assert!(!back.auto_detect.enabled);

        again.set_scoreboard_reader(ScoreboardReader::OcrV1);
        let off = toml::to_string_pretty(&again.to_config(&loaded)).unwrap();
        assert!(!off.contains("shadow_recognizer"), "{off}");
        assert!(off.contains("FrozenTear"), "{off}");
        assert!(off.contains("finished_game_close_secs = 240"), "{off}");
    }

    #[test]
    fn env_override_shows_locked_and_is_not_saved() {
        let base = base();
        assert!(!base.shadow_recognizer);
        for env in [Some("1"), Some("true"), Some("yes")] {
            let control = Config::shadow_control(base.shadow_recognizer, env);
            assert!(control.locked, "{env:?}");
            assert!(control.enabled(), "{env:?}");
            assert!(!control.file_on, "{env:?}");

            let mut form = SettingsForm::from_config_and_shadow(&base, control);
            assert_eq!(form.displayed_reader(), ScoreboardReader::Extra);
            assert_eq!(form.persisted_reader(), ScoreboardReader::OcrV1);
            assert!(!shadow_toggle_interactive(form.shadow.locked));

            form.set_toggle(SettingsToggle::ShadowRecognizer, true);
            form.set_toggle(SettingsToggle::ShadowRecognizer, false);
            form.set_scoreboard_reader(ScoreboardReader::Extra);
            assert_eq!(form.persisted_reader(), ScoreboardReader::OcrV1);
            assert_eq!(form.displayed_reader(), ScoreboardReader::Extra);

            form.player_name = "FrozenTear".into();
            let out = form.to_config(&base);
            assert!(!out.shadow_recognizer, "{env:?} must not be saved");
            assert_eq!(out.player_name.as_deref(), Some("FrozenTear"));
            assert_eq!(out.ocr_threads, Some(2));
            assert_eq!(out.data_dir, base.data_dir);
            let raw = toml::to_string_pretty(&out).unwrap();
            assert!(
                !raw.contains("shadow_recognizer"),
                "{env:?} leaked into the file: {raw}"
            );
        }
    }

    #[test]
    fn env_lock_does_not_clear_a_file_that_was_already_on() {
        let mut base = base();
        base.shadow_recognizer = true;
        let control = Config::shadow_control(true, Some("TRUE"));
        let mut form = SettingsForm::from_config_and_shadow(&base, control);
        assert!(form.shadow.locked);
        assert_eq!(form.displayed_reader(), ScoreboardReader::Extra);
        form.set_scoreboard_reader(ScoreboardReader::OcrV1);
        let out = form.to_config(&base);
        assert!(out.shadow_recognizer);
        let raw = toml::to_string_pretty(&out).unwrap();
        assert!(raw.contains("shadow_recognizer = true"), "{raw}");
    }

    #[test]
    fn save_keeps_name_reader_off_from_the_file() {
        let base: Config =
            toml::from_str("data_dir = \"/tmp/sst-name-reader\"\nname_reader = false\n")
                .expect("config with name_reader = false must parse");
        assert!(!base.name_reader);
        let form = SettingsForm::from_config_and_shadow(
            &base,
            Config::shadow_control(base.shadow_recognizer, None),
        );
        let out = form.to_config(&base);
        assert!(
            !out.name_reader,
            "Settings has no control for it, so the file value stays"
        );
        let raw = toml::to_string_pretty(&out).unwrap();
        assert!(raw.contains("name_reader = false"), "{raw}");
        assert!(form.to_config(&Config::default()).name_reader);
    }

    #[test]
    fn unchanged_save_keeps_commented_config_byte_for_byte() {
        let raw = "\
# local tracker config
data_dir = \"/tmp/sst-keep-comments\"

# scoreboard name
player_name = \"FrozenTear\"

session_window_secs = 900

# quiet time before a finished game is closed
finished_game_close_secs = 240

debug_ocr = true
ocr_threads = 2
game_process_names = [\"Overwatch.exe\", \"wine\"]

# extra number reader (private log only)
shadow_recognizer = true

[auto_detect]
# watch for match start and end
enabled = false
poll_interval_secs = 8
cooldown_secs = 60

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"
";
        let loaded: Config = toml::from_str(raw).expect("commented config must parse");
        assert!(loaded.shadow_recognizer);
        let form = SettingsForm::from_config_and_shadow(
            &loaded,
            Config::shadow_control(loaded.shadow_recognizer, None),
        );
        assert_eq!(form.displayed_reader(), ScoreboardReader::Extra);
        assert!(
            form.displayed_reader().file_flag(),
            "the checkbox is on when the file says shadow_recognizer = true"
        );
        assert!(!form.shadow.locked);
        assert_eq!(
            form.persisted_reader().id(),
            stat_tracker::shadow::digits::RECOGNIZER_ID
        );

        let saved =
            Config::text_for_save(Some(raw), &form.to_config(&loaded)).expect("unchanged save");
        assert_eq!(
            saved, raw,
            "an unchanged save must keep the file byte for byte"
        );
        assert!(saved.contains("# extra number reader (private log only)"));
        assert!(saved.contains("# scoreboard name"));
        assert!(saved.contains("# watch for match start and end"));
        assert!(saved.contains("finished_game_close_secs = 240"));
        assert!(saved.contains("not-a-real-token"));
    }

    #[test]
    fn flipping_the_toggle_changes_only_the_shadow_line() {
        let raw = "\
# hand-edited tracker config
data_dir = \"/tmp/sst-hand-edited\"

# scoreboard name
player_name = \"FrozenTear\"

# extra number reader (private log only)
shadow_recognizer = false  # keep this comment

session_window_secs = 1200
custom_note = \"leave this line alone\"
";
        let loaded: Config = toml::from_str(raw).expect("hand-edited config must parse");
        assert!(!loaded.shadow_recognizer);
        let mut form = SettingsForm::from_config_and_shadow(
            &loaded,
            Config::shadow_control(loaded.shadow_recognizer, None),
        );
        assert_eq!(form.displayed_reader(), ScoreboardReader::OcrV1);
        form.set_toggle(SettingsToggle::ShadowRecognizer, true);
        assert_eq!(form.displayed_reader(), ScoreboardReader::Extra);

        let saved =
            Config::text_for_save(Some(raw), &form.to_config(&loaded)).expect("toggle save");
        let before: Vec<_> = raw.split_inclusive('\n').collect();
        let after: Vec<_> = saved.split_inclusive('\n').collect();
        let diffs: Vec<_> = before
            .iter()
            .zip(after.iter())
            .filter(|(left, right)| left != right)
            .collect();
        assert_eq!(
            before.len(),
            after.len(),
            "the toggle must not add or drop a line:\n{saved}"
        );
        assert_eq!(
            diffs.len(),
            1,
            "one line should change, got {diffs:?}\n{saved}"
        );
        assert!(diffs[0].1.contains("shadow_recognizer = true"));
        assert!(diffs[0].1.contains("# keep this comment"));
        assert!(saved.contains("# hand-edited tracker config"));
        assert!(saved.contains("# scoreboard name"));
        assert!(saved.contains("custom_note = \"leave this line alone\""));
        assert!(saved.contains("player_name = \"FrozenTear\""));
        assert!(
            !saved.contains("game_process_names"),
            "the toggle must not add default keys: {saved}"
        );
    }

    #[test]
    fn toggling_a_missing_shadow_key_stays_on_the_root() {
        let original = "\
data_dir = \"/tmp/sst-settings-shape\"
capture_output = \"DP-1\"
player_name = \"FrozenTear\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"
";
        let tables = &original[original.find("[auto_detect]").expect("tables")..];
        let mut text = original.to_string();
        for on in [true, false, true] {
            let loaded = Config::parse_file_contents(&text).expect("parse");
            let mut form = SettingsForm::from_config_and_shadow(
                &loaded,
                Config::shadow_control(loaded.shadow_recognizer, None),
            );
            form.set_toggle(SettingsToggle::ShadowRecognizer, on);
            text = Config::text_for_save(Some(&text), &form.to_config(&loaded)).expect("toggle");
            let read = Config::parse_file_contents(&text).expect("load");
            assert_eq!(read.shadow_recognizer, on);
            assert_eq!(
                &text[text.find("[auto_detect]").expect("tables")..],
                tables,
                "tables changed while toggling {on}:\n{text}"
            );
            let header = text.find("[auto_detect]").expect("tables");
            assert_eq!(
                text[..header].matches("shadow_recognizer").count(),
                1,
                "one root key before the tables:\n{text}"
            );
            assert!(
                text[..header].contains(&format!("shadow_recognizer = {on}")),
                "{text}"
            );
        }
    }

    #[test]
    fn alpha_updates_checkbox_maps_to_update_channel_both_ways() {
        let mut base = base();
        let form = SettingsForm::from_config_and_shadow(&base, Config::shadow_control(false, None));
        assert!(!form.prerelease_updates, "default is stable");
        assert!(form.to_config(&base).update_channel.is_empty());

        for raw in ["prerelease", "PreRelease", " prerelease "] {
            base.update_channel = raw.to_string();
            let form =
                SettingsForm::from_config_and_shadow(&base, Config::shadow_control(false, None));
            assert!(form.prerelease_updates, "{raw:?} is the opt-in");
            assert_eq!(
                form.to_config(&base).update_channel,
                raw,
                "unchanged keeps spelling"
            );
        }

        base.update_channel.clear();
        let mut form =
            SettingsForm::from_config_and_shadow(&base, Config::shadow_control(false, None));
        form.set_toggle(SettingsToggle::PrereleaseUpdates, true);
        let on = form.to_config(&base);
        assert_eq!(on.update_channel, "prerelease");
        assert!(on.wants_prerelease_updates());

        let mut form =
            SettingsForm::from_config_and_shadow(&on, Config::shadow_control(false, None));
        assert!(form.prerelease_updates);
        form.set_toggle(SettingsToggle::PrereleaseUpdates, false);
        let off = form.to_config(&on);
        assert_eq!(off.update_channel, "stable");
        assert!(!off.wants_prerelease_updates());

        base.update_channel = "beta".to_string();
        let form = SettingsForm::from_config_and_shadow(&base, Config::shadow_control(false, None));
        assert!(!form.prerelease_updates, "anything else is stable");
        assert_eq!(form.to_config(&base).update_channel, "beta");
    }

    fn form_on(config: &Config, installed: Option<&str>) -> SettingsForm {
        SettingsForm::from_config_and_shadow(config, Config::shadow_control(false, None))
            .with_installed_version(config, installed)
    }

    #[test]
    fn alpha_updates_box_shows_the_channel_an_alpha_install_is_on() {
        let mut base = base();
        base.update_channel.clear();
        for alpha in ["0.5.0-alpha.3", "0.5.0-alpha.11", "0.5.0-rc1"] {
            assert!(
                form_on(&base, Some(alpha)).prerelease_updates,
                "{alpha} with update_channel unset shows ticked"
            );
        }
        for stable in [Some("0.4.24"), Some("0.5.0-test.1"), None] {
            assert!(
                !form_on(&base, stable).prerelease_updates,
                "{stable:?} with update_channel unset shows unticked"
            );
        }
        base.update_channel = "stable".into();
        assert!(!form_on(&base, Some("0.5.0-alpha.3")).prerelease_updates);
        base.update_channel = "prerelease".into();
        assert!(form_on(&base, Some("0.4.24")).prerelease_updates);
    }

    #[test]
    fn saving_settings_writes_update_channel_only_when_the_box_was_toggled() {
        let mut base = base();
        for (raw, installed) in [
            ("", Some("0.5.0-alpha.3")),
            ("", Some("0.4.24")),
            ("", None),
            ("stable", Some("0.5.0-alpha.3")),
            ("prerelease", Some("0.4.24")),
            ("beta", Some("0.5.0-alpha.3")),
        ] {
            base.update_channel = raw.into();
            let mut form = form_on(&base, installed);
            assert_eq!(
                form.to_config(&base).update_channel,
                raw,
                "untouched save keeps {raw:?} on {installed:?}"
            );
            // Another field changes, the box does not.
            form.debug_ocr = !form.debug_ocr;
            assert_eq!(form.to_config(&base).update_channel, raw);
            // Toggled and toggled back is untouched too.
            let shown = form.prerelease_updates;
            form.set_toggle(SettingsToggle::PrereleaseUpdates, !shown);
            form.set_toggle(SettingsToggle::PrereleaseUpdates, shown);
            assert_eq!(form.to_config(&base).update_channel, raw);
        }

        // Alpha install, unset channel: the box shows ticked, and unticking
        // it is the only way `stable` gets written.
        base.update_channel.clear();
        let mut form = form_on(&base, Some("0.5.0-alpha.3"));
        assert!(form.prerelease_updates);
        form.set_toggle(SettingsToggle::PrereleaseUpdates, false);
        let off = form.to_config(&base);
        assert_eq!(off.update_channel, "stable");
        let reloaded = form_on(&off, Some("0.5.0-alpha.3"));
        assert!(!reloaded.prerelease_updates, "stable sticks after reload");

        // Release install, unset channel: ticking writes `prerelease`.
        let mut form = form_on(&base, Some("0.4.24"));
        assert!(!form.prerelease_updates);
        form.set_toggle(SettingsToggle::PrereleaseUpdates, true);
        assert_eq!(form.to_config(&base).update_channel, "prerelease");
    }

    #[test]
    fn alpha_updates_toggle_preserves_every_other_key() {
        let raw = "\
# hand-edited tracker config
data_dir = \"/tmp/sst-alpha-toggle\"
player_name = \"FrozenTear\"
session_window_secs = 900
finished_game_close_secs = 240
debug_ocr = true
ocr_threads = 2
shadow_recognizer = true  # keep this comment
reader = \"new\"
setup_completed = true
game_process_names = [\"Overwatch.exe\"]

[auto_detect]
enabled = false
poll_interval_secs = 8
cooldown_secs = 60

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"
";
        let mut text = raw.to_string();
        for (on, word) in [
            (true, "prerelease"),
            (false, "stable"),
            (true, "prerelease"),
        ] {
            let loaded = Config::parse_file_contents(&text).expect("parse");
            let mut form = SettingsForm::from_config_and_shadow(
                &loaded,
                Config::shadow_control(loaded.shadow_recognizer, None),
            );
            assert_eq!(form.prerelease_updates, !on);
            form.set_toggle(SettingsToggle::PrereleaseUpdates, on);
            let next = form.to_config(&loaded);
            let mut rest = next.clone();
            rest.update_channel.clone_from(&loaded.update_channel);
            assert_eq!(rest, loaded, "only update_channel may change");

            text = Config::text_for_save(Some(&text), &next).expect("save");
            assert!(
                text.contains(&format!("update_channel = \"{word}\"")),
                "{text}"
            );
            let read = Config::parse_file_contents(&text).expect("reload");
            assert_eq!(read.wants_prerelease_updates(), on);
            assert_eq!(read.data_dir, loaded.data_dir);
            assert_eq!(read.ocr_threads, Some(2));
            assert_eq!(read.finished_game_close_secs, 240);
            assert_eq!(read.session_window_secs, 900);
            assert!(read.debug_ocr);
            assert!(read.shadow_recognizer);
            assert!(read.reader.is_new());
            assert!(read.setup_completed);
            assert!(!read.auto_detect.enabled);
            assert_eq!(read.player_name.as_deref(), Some("FrozenTear"));
            assert_eq!(
                read.sync.as_ref().map(|s| s.token.as_str()),
                Some("not-a-real-token")
            );
            for line in raw.lines() {
                assert!(text.contains(line), "lost {line:?}:\n{text}");
            }
        }
    }

    #[test]
    fn alpha_updates_copy_is_plain_and_does_not_ask_for_a_restart() {
        for copy in [
            PRERELEASE_UPDATES_LABEL,
            PRERELEASE_UPDATES_HINT,
            PRERELEASE_UPDATES_APPLIES,
        ] {
            assert!(!copy.contains('—'), "{copy}");
            assert!(!copy.contains('–'), "{copy}");
        }
        assert_eq!(PRERELEASE_UPDATES_LABEL, "Get alpha updates");
        assert!(PRERELEASE_UPDATES_HINT.contains("alphas and release candidates"));
        assert!(PRERELEASE_UPDATES_HINT.contains("only stable releases"));
        assert!(PRERELEASE_UPDATES_APPLIES.contains("No restart needed"));
        assert!(!PRERELEASE_UPDATES_HINT.contains("Restart"));
    }

    // ------------------------------------------------ classic OCR switch

    const ALL_PACKS: PackReadiness = PackReadiness {
        heroes: true,
        maps: true,
        words: true,
        result_words: true,
        phase_titles: true,
        end_titles: true,
    };
    const NO_MAPS: PackReadiness = PackReadiness {
        maps: false,
        ..ALL_PACKS
    };
    /// A tracker version that reads everything without Tesseract.
    const NO_READS: &[OcrRead] = &[];

    #[test]
    fn classic_ocr_is_on_by_default_and_saves_as_the_ocr_key() {
        let loaded = base();
        let mut form =
            SettingsForm::from_config_and_shadow(&loaded, Config::shadow_control(false, None));
        assert!(
            form.classic_ocr,
            "a config without `ocr` shows the switch on"
        );
        assert_eq!(
            form.to_config(&loaded),
            loaded,
            "an untouched form saves nothing"
        );

        form.set_toggle(SettingsToggle::ClassicOcr, false);
        let off = form.to_config(&loaded);
        assert_eq!(off.ocr, OcrSetting::Off);
        // Off implies the new reader; it does not rewrite `reader`.
        assert_eq!(off.reader, loaded.reader);
        let mut rest = off.clone();
        rest.ocr = OcrSetting::On;
        assert_eq!(rest, loaded, "only `ocr` may change");

        let reloaded =
            SettingsForm::from_config_and_shadow(&off, Config::shadow_control(false, None));
        assert!(!reloaded.classic_ocr);
        let mut back = reloaded.clone();
        back.set_toggle(SettingsToggle::ClassicOcr, true);
        assert_eq!(back.to_config(&off).ocr, OcrSetting::On);
    }

    #[test]
    fn turning_classic_ocr_off_is_refused_without_every_reader_pack() {
        assert_eq!(classic_ocr_toggle(false, &ALL_PACKS, NO_READS), Ok(false));
        let refused = classic_ocr_toggle(false, &NO_MAPS, NO_READS).unwrap_err();
        assert!(refused.starts_with("Classic OCR stays on"), "{refused}");
        assert!(
            refused.contains("the map pack is not installed"),
            "{refused}"
        );
        assert!(
            refused.ends_with(ocr_switch::CLICK_FIX),
            "a click is told to turn it off again: {refused}"
        );
        assert!(
            classic_ocr_toggle(false, &PackReadiness::default(), NO_READS).is_err(),
            "no packs at all"
        );
        // Turning it on never needs a pack or a newer tracker.
        assert_eq!(
            classic_ocr_toggle(true, &PackReadiness::default(), &OcrRead::ALL),
            Ok(true)
        );
        assert_eq!(classic_ocr_toggle(true, &ALL_PACKS, NO_READS), Ok(true));
    }

    #[test]
    fn turning_classic_ocr_off_is_refused_while_the_tracker_needs_it_for_a_read() {
        // Every pack is there: the tracker version is what stands in the way.
        let refused = classic_ocr_toggle(false, &ALL_PACKS, &[OcrRead::StatCells]).unwrap_err();
        assert_eq!(
            refused,
            format!(
                "Classic OCR stays on: this tracker version cannot read the scoreboard numbers without it yet. {}",
                ocr_switch::LATER_VERSION
            )
        );
        assert!(
            !refused.contains("To clear this message"),
            "a click has nothing to clear: {refused}"
        );
        assert!(
            !refused.contains("download"),
            "no pack would turn it off: {refused}"
        );
        // What this build says for a click today follows the build.
        assert_eq!(
            classic_ocr_toggle(false, &ALL_PACKS, &OcrRead::needing_ocr()).is_ok(),
            OcrRead::needing_ocr().is_empty()
        );
    }

    #[test]
    fn a_save_checks_only_when_it_turns_classic_ocr_off() {
        let refusal = classic_ocr_toggle(false, &NO_MAPS, NO_READS).err();
        assert!(refusal.is_some());
        // on stays on, off stays off, off goes on: nothing to check
        for (saved, form_classic) in [
            (OcrSetting::On, true),
            (OcrSetting::Off, false),
            (OcrSetting::Off, true),
        ] {
            assert_eq!(
                ocr_save_problem(saved, form_classic, Some(&NO_MAPS), &OcrRead::ALL),
                None,
                "{saved:?} to classic={form_classic}"
            );
        }
        // on goes off: refused with the reason, or let through
        assert_eq!(
            ocr_save_problem(OcrSetting::On, false, Some(&NO_MAPS), NO_READS),
            refusal
        );
        assert_eq!(
            ocr_save_problem(OcrSetting::On, false, Some(&ALL_PACKS), NO_READS),
            None
        );
        assert!(
            ocr_save_problem(OcrSetting::On, false, Some(&ALL_PACKS), &[OcrRead::OwnRow]).is_some()
        );
        // the packs were never looked at: the save goes through, and the
        // tracker decides when it starts
        assert_eq!(
            ocr_save_problem(OcrSetting::On, false, None, NO_READS),
            None
        );
    }

    #[test]
    fn a_shown_refusal_goes_away_once_the_packs_are_there() {
        let shown = classic_ocr_toggle(false, &NO_MAPS, NO_READS).err();
        assert_eq!(
            refusal_after_pack_check(shown.clone(), &ALL_PACKS, NO_READS),
            None,
            "3 of 3 and a refusal are not shown together"
        );
        assert_eq!(
            refusal_after_pack_check(shown.clone(), &NO_MAPS, NO_READS),
            shown
        );
        // a look at the packs does not invent a refusal nobody clicked for
        assert_eq!(refusal_after_pack_check(None, &NO_MAPS, NO_READS), None);
    }

    #[test]
    fn the_card_follows_what_the_tracker_runs_not_the_box() {
        let all = OcrCardBlocks {
            reading_model: true,
            new_reader: true,
        };
        assert_eq!(ocr_card_blocks(false), all);
        assert_eq!(
            ocr_card_blocks(true),
            OcrCardBlocks {
                reading_model: false,
                new_reader: false
            }
        );
        // Running with OCR off: only then are the blocks hidden.
        let off = switch_status(41, None, ALL_PACKS);
        assert!(daemon_runs_ocr_off(Some(&off), Some(41)));
        // Refused at start: Tesseract runs, the model block stays.
        let refused = switch_status(41, refusal_for(&NO_MAPS), NO_MAPS);
        assert!(!daemon_runs_ocr_off(Some(&refused), Some(41)));
        // Another run's file, a stopped tracker, no file.
        assert!(!daemon_runs_ocr_off(Some(&off), Some(42)));
        assert!(!daemon_runs_ocr_off(Some(&off), None));
        assert!(!daemon_runs_ocr_off(None, Some(41)));
    }

    #[test]
    fn the_pack_line_counts_ready_packs() {
        assert_eq!(pack_status_line(None), READER_PACKS_UNCHECKED);
        assert_eq!(
            pack_status_line(Some(&ALL_PACKS)),
            "Reader packs ready: 3 of 3"
        );
        assert_eq!(
            pack_status_line(Some(&NO_MAPS)),
            "Reader packs ready: 2 of 3"
        );
        assert_eq!(
            pack_status_line(Some(&PackReadiness::default())),
            "Reader packs ready: 0 of 3"
        );
    }

    fn refusal_for(packs: &PackReadiness) -> Option<ocr_switch::Refusal> {
        ocr_switch::decide(OcrSetting::Off, packs, NO_READS).refusal
    }

    fn switch_status(
        pid: u32,
        refusal: Option<ocr_switch::Refusal>,
        packs: PackReadiness,
    ) -> OcrSwitchStatus {
        OcrSwitchStatus {
            pid,
            requested: OcrSetting::Off,
            effective: if refusal.is_some() {
                OcrSetting::On
            } else {
                OcrSetting::Off
            },
            refusal,
            packs: Some(packs),
            blocked_total: 0,
            blocked: Vec::new(),
            leptess_created: 0,
            share_open: false,
            share_notice: String::new(),
            capture_notice: String::new(),
        }
    }

    #[test]
    fn kde_direct_renders_only_when_saved_or_chosen() {
        for route in [
            CaptureRoute::Auto,
            CaptureRoute::Screencast,
            CaptureRoute::Screenshot,
        ] {
            assert!(!render_wanted(route, false), "{route:?} at launch");
            assert!(render_wanted(route, true), "{route:?} then the chip");
        }
        assert!(render_wanted(CaptureRoute::Kwin, false));
        assert!(render_wanted(CaptureRoute::Kwin, true));
        // Flipping the default-on constant makes automatic render too, and
        // nothing else.
        assert!(render_wanted_with(CaptureRoute::Auto, false, true));
        assert!(!render_wanted_with(CaptureRoute::Auto, false, false));
        assert!(!render_wanted_with(CaptureRoute::Screencast, false, true));
        assert!(!render_wanted_with(CaptureRoute::Screenshot, false, true));
    }

    /// `app.rs` with whitespace removed.
    fn squashed_app() -> String {
        include_str!("app.rs").split_whitespace().collect()
    }

    // The app's three KDE direct checks (#433 review): launch and the
    // recheck after a restart or a closed share decide on the saved route
    // alone; only a click on the KDE direct chip adds a render.

    #[test]
    fn the_app_has_three_kde_direct_checks() {
        assert_eq!(
            squashed_app()
                .matches("stat_tracker::capture::kwin_direct_status(")
                .count(),
            3,
            "launch, chip click, recheck"
        );
    }

    #[test]
    fn the_launch_check_renders_only_for_the_saved_route() {
        assert!(squashed_app().contains(
            "kwin_direct_status(app.data_dir.clone(),settings::render_wanted(app.saved_config.capture_route,false),)"
        ));
    }

    #[test]
    fn the_recheck_renders_only_for_the_saved_route() {
        assert!(squashed_app().contains(
            "ifstd::mem::take(&mutself.kde_direct_recheck){tasks.push(Task::perform(stat_tracker::capture::kwin_direct_status(self.data_dir.clone(),settings::render_wanted(self.saved_config.capture_route,false),"
        ));
    }

    #[test]
    fn the_chip_click_check_renders_for_the_chip() {
        let app = squashed_app();
        assert!(app.contains(
            "letchip_clicked=route==stat_tracker::config::CaptureRoute::Kwin;ifchip_clicked&&self.fixture.is_none(){"
        ));
        assert!(app.contains(
            "kwin_direct_status(self.data_dir.clone(),settings::render_wanted(self.saved_config.capture_route,chip_clicked),)"
        ));
    }

    #[test]
    fn kde_direct_chip_shows_on_kde_or_when_saved() {
        assert!(show_kde_direct("KDE", CaptureRoute::Auto));
        assert!(show_kde_direct("plasma:KDE", CaptureRoute::Screencast));
        assert!(!show_kde_direct("GNOME", CaptureRoute::Auto));
        assert!(!show_kde_direct("niri", CaptureRoute::Screenshot));
        assert!(show_kde_direct("niri", CaptureRoute::Kwin));
    }

    #[test]
    fn session_kind_follows_the_tracker_rule() {
        use SessionKind::*;
        assert_eq!(SessionKind::from_env(Some("wayland"), false), Wayland);
        assert_eq!(SessionKind::from_env(Some("Wayland"), true), Wayland);
        assert_eq!(SessionKind::from_env(Some("x11"), false), X11);
        assert_eq!(SessionKind::from_env(Some("X11"), false), X11);
        assert_eq!(SessionKind::from_env(None, true), Wayland);
        assert_eq!(SessionKind::from_env(Some("tty"), true), Wayland);
        // Not known: x11 beside a stray WAYLAND_DISPLAY, or no evidence.
        assert_eq!(SessionKind::from_env(Some("x11"), true), Unknown);
        for kind in [None, Some("tty"), Some("unspecified")] {
            assert_eq!(SessionKind::from_env(kind, false), Unknown, "{kind:?}");
        }
    }

    #[test]
    fn capture_chips_on_wayland_or_an_unknown_session_are_unchanged() {
        use CaptureRoute::*;
        for session in [SessionKind::Wayland, SessionKind::Unknown] {
            assert_eq!(
                capture_route_chips("GNOME", session, Auto),
                vec![Auto, Screencast, Screenshot]
            );
            assert_eq!(
                capture_route_chips("KDE", session, Screencast),
                vec![Auto, Screencast, Screenshot, Kwin]
            );
            assert_eq!(
                capture_route_chips("niri", session, Kwin),
                vec![Auto, Screencast, Screenshot, Kwin]
            );
            assert!(capture_route_help(session).starts_with("Automatic uses the screen stream"));
            assert!(capture_route_notes("i3", session, Auto, None, None).is_empty());
            assert!(capture_route_notes("i3", session, Screencast, None, None).is_empty());
        }
        assert_eq!(capture_route_label(Auto), "Automatic");
        assert_eq!(capture_route_label(Screencast), "Screen stream");
        assert_eq!(capture_route_label(Screenshot), "Older screenshot");
        assert_eq!(capture_route_label(Kwin), "KDE direct");
    }

    #[test]
    fn capture_chips_on_x11_drop_the_wayland_only_methods() {
        use CaptureRoute::*;
        let x11 = SessionKind::X11;
        // Plasma X11 included: KDE direct needs Wayland too. Older
        // screenshot (the portal) runs on X11.
        for desktop in ["KDE", "GNOME", "i3", ""] {
            assert_eq!(
                capture_route_chips(desktop, x11, Auto),
                vec![Auto, Screenshot]
            );
            assert_eq!(
                capture_route_chips(desktop, x11, Screenshot),
                vec![Auto, Screenshot]
            );
        }
        // A saved Wayland-only route stays visible so its warning has a chip.
        assert_eq!(
            capture_route_chips("GNOME", x11, Screencast),
            vec![Auto, Screencast, Screenshot]
        );
        assert_eq!(
            capture_route_chips("KDE", x11, Kwin),
            vec![Auto, Screenshot, Kwin]
        );
        assert!(capture_route_help(x11).contains("X11"));
        assert!(capture_route_help(x11).contains("Older screenshot"));
        assert!(!capture_route_help(x11).contains("Screen stream"));
    }

    #[test]
    fn capture_notes_on_x11() {
        use stat_tracker::capture::KwinProbe;
        let x11 = SessionKind::X11;
        // Automatic: the Older screenshot note only, never KDE direct's.
        let missing = KwinProbe::Unavailable("no KWin".into());
        assert_eq!(
            capture_route_notes("KDE", x11, CaptureRoute::Auto, Some(&missing), None),
            vec![(X11_SCREENSHOT_NOTE.to_string(), NoteTone::Info)]
        );
        assert!(X11_SCREENSHOT_NOTE.contains("whole desktop"));
        assert!(X11_SCREENSHOT_NOTE.contains("Automatic is recommended"));
        // Older screenshot saved: no Wayland warning, its note is a warning.
        let notes = capture_route_notes("i3", x11, CaptureRoute::Screenshot, None, None);
        assert_eq!(
            notes,
            vec![(X11_SCREENSHOT_NOTE.to_string(), NoteTone::Warn)]
        );
        // GNOME X11 still flashes.
        let notes = capture_route_notes("GNOME", x11, CaptureRoute::Auto, None, None);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[1].0, GNOME_SCREENSHOT_NOTE);
        for (saved, label) in [
            (CaptureRoute::Screencast, "Screen stream"),
            (CaptureRoute::Kwin, "KDE direct"),
        ] {
            let notes = capture_route_notes("KDE", x11, saved, Some(&missing), None);
            assert_eq!(notes.len(), 2, "{notes:?}");
            assert_eq!(notes[0], (x11_route_warning(saved), NoteTone::Warn));
            assert!(notes[0].0.starts_with(label), "{}", notes[0].0);
            assert!(notes[0].0.contains("only on Wayland"));
            assert!(notes[0].0.contains("uses Automatic"));
            assert_eq!(notes[1], (X11_SCREENSHOT_NOTE.to_string(), NoteTone::Info));
        }
        // The tracker's notice about the same thing is not said twice; any
        // other notice is.
        let same = "the saved capture method, the screen stream, needs a Wayland session and this one is X11; automatic is used: using X11 instead";
        let notes = capture_route_notes("i3", x11, CaptureRoute::Screencast, None, Some(same));
        assert!(!notes.iter().any(|(n, _)| n.starts_with("Tracker:")));
        let other = "X11 capture failed: no display";
        let notes = capture_route_notes("i3", x11, CaptureRoute::Screencast, None, Some(other));
        assert_eq!(notes.last().unwrap().0, format!("Tracker: {other}."));
        let notes = capture_route_notes("i3", x11, CaptureRoute::Auto, None, Some(same));
        assert!(notes.last().unwrap().0.starts_with("Tracker: the saved"));
        // Kept when it says nothing else works: that is news.
        let nothing = "the saved capture method, the screen stream, needs a Wayland session and this one is X11; automatic is used: no other capture method works here";
        let notes = capture_route_notes("i3", x11, CaptureRoute::Screencast, None, Some(nothing));
        assert_eq!(notes.last().unwrap().0, format!("Tracker: {nothing}."));
    }

    /// The portal notices end in a period already: one is kept, not two.
    #[test]
    fn a_tracker_notice_that_ends_a_sentence_gets_one_period() {
        use stat_tracker::capture::{PORTAL_PROMPT_NOTICE, PORTAL_STALL_NOTICE};
        for notice in [PORTAL_PROMPT_NOTICE, PORTAL_STALL_NOTICE] {
            assert!(notice.ends_with('.'), "{notice}");
            for session in [SessionKind::Wayland, SessionKind::X11] {
                let notes = capture_route_notes(
                    "KDE",
                    session,
                    CaptureRoute::Screenshot,
                    None,
                    Some(notice),
                );
                let line = &notes.last().unwrap().0;
                assert_eq!(*line, format!("Tracker: {notice}"), "{session:?}");
                assert!(!line.ends_with(".."), "{line}");
            }
        }
        assert_eq!(
            tracker_note("no capture method"),
            "Tracker: no capture method."
        );
        assert_eq!(tracker_note("route. Stall.  "), "Tracker: route. Stall.");
    }

    #[test]
    fn install_row_for_a_package_has_no_command_to_copy() {
        use crate::uninstall::{PackageManager, PackageOwner};
        let aur = PackageOwner {
            manager: PackageManager::Pacman,
            package: "scuffed-stat-tracker-bin".into(),
        };
        let row = install_row(
            update::PackageProbe::Done(Some(&aur)),
            Some("0.5.0"),
            Some("0.5.0-rc.7"),
        );
        assert!(!row.copy);
        assert!(!row.line.contains("curl"));
        assert!(row.line.contains("paru -Syu"));
        // A script install keeps the pinned command and its Copy button.
        let row = install_row(
            update::PackageProbe::Pending,
            Some("0.5.0"),
            Some("0.5.0-rc.7"),
        );
        assert!(!row.copy);
        assert_eq!(row.line, update::PACKAGE_PROBE_PENDING);
        let row = install_row(
            update::PackageProbe::Done(None),
            Some("0.5.0"),
            Some("0.5.0-rc.7"),
        );
        assert!(row.copy);
        assert_eq!(row.caption, "Install / update command");
        assert_eq!(row.line, update::install_command_for(Some("0.5.0"), None));
    }

    #[test]
    fn new_reader_switch_is_no_longer_called_alpha() {
        assert_eq!(NEW_READER_LABEL, "New reader");
        assert!(!NEW_READER_LABEL.contains("alpha"));
    }

    #[test]
    fn capture_notes_warn_about_the_gnome_flash_and_a_missing_kde_direct() {
        use stat_tracker::capture::KwinProbe;
        // GNOME: the flash is always said, as a warning once chosen.
        let notes = capture_route_notes(
            "ubuntu:GNOME",
            SessionKind::Wayland,
            CaptureRoute::Auto,
            None,
            None,
        );
        assert_eq!(
            notes,
            vec![(GNOME_SCREENSHOT_NOTE.to_string(), NoteTone::Info)]
        );
        assert!(GNOME_SCREENSHOT_NOTE.contains("flashes the screen"));
        assert!(GNOME_SCREENSHOT_NOTE.contains("sound"));
        let chosen = capture_route_notes(
            "GNOME",
            SessionKind::Wayland,
            CaptureRoute::Screenshot,
            None,
            None,
        );
        assert_eq!(chosen[0].1, NoteTone::Warn);
        // Not on KDE.
        assert!(
            capture_route_notes(
                "KDE",
                SessionKind::Wayland,
                CaptureRoute::Screenshot,
                None,
                None
            )
            .is_empty()
        );
        // KDE direct there: nothing to say. Not there: the reason.
        assert!(
            capture_route_notes(
                "KDE",
                SessionKind::Wayland,
                CaptureRoute::Kwin,
                Some(&KwinProbe::Available),
                None
            )
            .is_empty()
        );
        let missing = KwinProbe::Unavailable("KWin refused the call".into());
        let notes = capture_route_notes(
            "KDE",
            SessionKind::Wayland,
            CaptureRoute::Auto,
            Some(&missing),
            None,
        );
        assert_eq!(notes.len(), 1);
        assert!(notes[0].0.contains("KWin refused the call"));
        assert_eq!(notes[0].1, NoteTone::Info);
        let notes = capture_route_notes(
            "KDE",
            SessionKind::Wayland,
            CaptureRoute::Kwin,
            Some(&missing),
            Some("KDE direct capture is not available (x); using screen stream instead"),
        );
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|(_, tone)| *tone == NoteTone::Warn));
        assert!(notes[1].0.starts_with("Tracker: KDE direct"));
        assert!(KDE_DIRECT_HELP.contains("no prompt or indicator"));
        assert!(KDE_DIRECT_HELP.contains("never saved unless a debug option is on"));
        // Not checked is said plainly, never as a warning, even when chosen.
        let not_checked = KwinProbe::NotChecked("needs 6.8.1".into());
        let notes = capture_route_notes(
            "KDE",
            SessionKind::Wayland,
            CaptureRoute::Kwin,
            Some(&not_checked),
            None,
        );
        assert_eq!(notes.len(), 1);
        assert!(notes[0].0.starts_with("KDE direct was not checked"));
        assert!(!notes[0].0.contains("not available"));
        assert_eq!(notes[0].1, NoteTone::Info);
        let notes = capture_route_notes(
            "KDE",
            SessionKind::Wayland,
            CaptureRoute::Kwin,
            Some(&KwinProbe::NotProbed),
            None,
        );
        assert_eq!(
            notes,
            vec![(KDE_DIRECT_NOT_CHECKED.to_string(), NoteTone::Info)]
        );
    }

    #[test]
    fn kde_direct_is_checked_again_after_a_restart_or_a_closed_share() {
        // First sight of a running tracker, and a restart (new pid).
        assert!(kde_direct_recheck_due((None, false), (Some(41), false)));
        assert!(kde_direct_recheck_due((Some(41), false), (Some(42), false)));
        // The share closed.
        assert!(kde_direct_recheck_due((Some(41), true), (Some(41), false)));
        // Nothing changed, the share opened, or the tracker stopped.
        assert!(!kde_direct_recheck_due(
            (Some(41), false),
            (Some(41), false)
        ));
        assert!(!kde_direct_recheck_due((Some(41), false), (Some(41), true)));
        assert!(!kde_direct_recheck_due((Some(41), false), (None, false)));
        let mut status = switch_status(41, None, NO_MAPS);
        status.share_open = true;
        assert!(daemon_share_open(Some(&status), Some(41)));
        assert!(!daemon_share_open(Some(&status), Some(42)));
    }

    #[test]
    fn the_capture_notice_is_the_running_trackers_only() {
        let mut status = switch_status(41, None, NO_MAPS);
        assert_eq!(daemon_capture_notice(Some(&status), Some(41)), None);
        status.capture_notice = "KDE direct capture is not available".into();
        assert_eq!(
            daemon_capture_notice(Some(&status), Some(41)).as_deref(),
            Some("KDE direct capture is not available")
        );
        assert_eq!(daemon_capture_notice(Some(&status), Some(42)), None);
    }

    #[test]
    fn the_tracker_notice_is_shown_only_for_the_tracker_that_is_running() {
        let held = switch_status(41, None, NO_MAPS);
        let banner = NO_MAPS.hold_banner();
        assert!(
            banner
                .as_deref()
                .is_some_and(|b| b.starts_with("Reading on hold"))
        );
        assert_eq!(daemon_ocr_notice(Some(&held), Some(41)), banner);
        // another run's status file, a stopped tracker, no file
        assert_eq!(daemon_ocr_notice(Some(&held), Some(42)), None);
        assert_eq!(daemon_ocr_notice(Some(&held), None), None);
        assert_eq!(daemon_ocr_notice(None, Some(41)), None);
        // refused at startup: the reason, worded for a tracker that has to
        // be restarted, not for a click
        let refused = switch_status(41, refusal_for(&NO_MAPS), NO_MAPS);
        let notice = daemon_ocr_notice(Some(&refused), Some(41)).expect("the reason");
        assert!(notice.ends_with(ocr_switch::DAEMON_FIX), "{notice}");
        assert!(!notice.contains("off again"), "{notice}");
        // off with every pack: nothing to show
        let fine = switch_status(41, None, ALL_PACKS);
        assert_eq!(daemon_ocr_notice(Some(&fine), Some(41)), None);
    }

    #[test]
    fn a_declined_share_is_shown_for_the_running_tracker() {
        let mut status = switch_status(41, None, ALL_PACKS);
        status.share_notice =
            "Screen share was declined: no captures until Overwatch is restarted".into();
        assert_eq!(
            daemon_share_notice(Some(&status), Some(41)).as_deref(),
            Some(status.share_notice.as_str())
        );
        assert_eq!(daemon_share_notice(Some(&status), Some(42)), None);
        assert_eq!(daemon_share_notice(Some(&status), None), None);
        assert_ne!(
            daemon_health_notice(Some(&status), Some(41)).as_deref(),
            Some(status.share_notice.as_str())
        );
        status.share_notice.clear();
        assert_eq!(daemon_share_notice(Some(&status), Some(41)), None);
    }

    /// Tracker health takes a hold, not a refusal: a tracker that refused
    /// off reads everything with classic OCR, and health stays "Ready".
    #[test]
    fn a_refused_off_stays_out_of_tracker_health() {
        let refused = switch_status(41, refusal_for(&NO_MAPS), NO_MAPS);
        assert!(daemon_ocr_notice(Some(&refused), Some(41)).is_some());
        assert_eq!(daemon_health_notice(Some(&refused), Some(41)), None);
        // refused because this tracker version needs OCR (a hand-edited off)
        let mut for_build = switch_status(41, None, ALL_PACKS);
        for_build.effective = OcrSetting::On;
        for_build.refusal =
            ocr_switch::decide(OcrSetting::Off, &ALL_PACKS, &[OcrRead::StatCells]).refusal;
        let notice = daemon_ocr_notice(Some(&for_build), Some(41)).expect("Settings says why");
        assert!(
            notice.ends_with(ocr_switch::TURN_BACK_ON),
            "and how to clear it: {notice}"
        );
        assert_eq!(daemon_health_notice(Some(&for_build), Some(41)), None);
        // running with OCR off and a pack gone: that is a health problem
        let held = switch_status(41, None, NO_MAPS);
        assert_eq!(
            daemon_health_notice(Some(&held), Some(41)),
            NO_MAPS.hold_banner()
        );
        assert_eq!(daemon_health_notice(Some(&held), Some(42)), None);
    }

    #[test]
    fn the_card_does_not_describe_an_off_this_version_cannot_give() {
        let can = ocr_hint_lines(NO_READS);
        assert_eq!(can.hint, CLASSIC_OCR_HINT);
        assert!(can.restart_and_packs);
        // where the member decides, the cost of off is said
        assert!(
            CLASSIC_OCR_HINT.contains(
                "the map name on the scoreboard has no second reader: a game whose map is \
                 never read is saved without a map until you pick one"
            ),
            "{CLASSIC_OCR_HINT}"
        );
        // no rate is claimed for it: against OCR the map is not missing
        // more often in the members' model re-run
        assert!(
            !CLASSIC_OCR_HINT.contains("more often"),
            "{CLASSIC_OCR_HINT}"
        );
        assert!(
            CLASSIC_OCR_HINT.contains("A game that ends with a stat never read is not saved."),
            "{CLASSIC_OCR_HINT}"
        );
        assert!(
            CLASSIC_OCR_HINT.contains(
                "a co-op event scoreboard (Junkenstein) can be saved as a game when its map \
                 name is not read"
            ),
            "{CLASSIC_OCR_HINT}"
        );
        assert!(
            CLASSIC_OCR_HINT.contains(
                "A scoreboard is not saved when your row has never been highlighted, your \
                 hero icon is dead or covered, and the tracker has not learned your name yet."
            ),
            "{CLASSIC_OCR_HINT}"
        );
        let cannot = ocr_hint_lines(&[OcrRead::StatCells]);
        assert_eq!(cannot.hint, CLASSIC_OCR_HINT_NOT_YET);
        assert!(
            !cannot.restart_and_packs,
            "no pack count for a switch that cannot move"
        );
        for word in ["Off:", "CPU", "never", "less"] {
            assert!(
                !CLASSIC_OCR_HINT_NOT_YET.contains(word),
                "{CLASSIC_OCR_HINT_NOT_YET}"
            );
        }
        // nothing in either hint is a number nobody measured
        assert!(!CLASSIC_OCR_HINT.contains("CPU"), "{CLASSIC_OCR_HINT}");
        // what this version shows follows what it can do
        assert_eq!(
            ocr_hint_lines(&OcrRead::needing_ocr()).restart_and_packs,
            OcrRead::needing_ocr().is_empty()
        );
    }

    #[test]
    fn one_problem_line_and_only_a_refused_click_can_be_dismissed() {
        assert_eq!(card_problem(None, None), None);
        assert_eq!(card_problem(None, Some("hold")), Some(("hold", false)));
        assert_eq!(card_problem(Some("no"), None), Some(("no", true)));
        // both: the click's line, once (after a hand-edited off they are
        // the same sentence)
        assert_eq!(card_problem(Some("no"), Some("no")), Some(("no", true)));
        assert_eq!(card_problem(Some("no"), Some("hold")), Some(("no", true)));
    }

    #[test]
    fn classic_ocr_copy_is_plain() {
        assert_eq!(CLASSIC_OCR_LABEL, "Use classic OCR (Tesseract)");
        for copy in [
            CLASSIC_OCR_LABEL,
            CLASSIC_OCR_HINT,
            CLASSIC_OCR_HINT_NOT_YET,
            CLASSIC_OCR_RESTART,
            NEW_READER_IMPLIED,
            READER_PACKS_UNCHECKED,
            ocr_switch::CLICK_FIX,
            ocr_switch::DAEMON_FIX,
            ocr_switch::LATER_VERSION,
            ocr_switch::TURN_BACK_ON,
            ocr_switch::HOLD_FIX,
            ocr_switch::RESTART_TO_APPLY,
        ] {
            assert!(!copy.contains('—'), "{copy}");
            assert!(!copy.contains('–'), "{copy}");
        }
        assert!(CLASSIC_OCR_HINT.contains("does not start Tesseract"));
        // "Tesseract" in the label is explained by the first words under it.
        for hint in [CLASSIC_OCR_HINT, CLASSIC_OCR_HINT_NOT_YET] {
            assert!(hint.starts_with("Tesseract is the classic text reader."));
        }
        assert!(CLASSIC_OCR_RESTART.contains("Restart"));
    }
}

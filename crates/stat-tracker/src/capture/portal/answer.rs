//! Waiting for the screenshot portal's answer: the checks on an
//! unanswered request (who owns the portal now, the screenshot permission),
//! the verdict, and when a request is warned about or given up.

use super::backoff::{SHOT_UNCLOSED_HOLD, ShotBackoff};
use super::{PORTAL_DESKTOP, lock};

/// Who owns `org.freedesktop.portal.Desktop` now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnerLookup {
    Owner(String),
    /// `NameHasNoOwner`: no portal is running.
    NoOwner,
    /// A timeout or another error: nothing is known.
    Unknown,
}

/// The permission store's `screenshot` entry for unsandboxed apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Permission {
    Yes,
    /// No entry (`NotFound`, or no row for app id `""`).
    Absent,
    /// Anything else: "no", "ask", or the store could not be read.
    Other,
}

/// What [`await_answer`] needs from the desktop; a fake in tests.
pub(crate) trait ShotChecks: Sync {
    fn portal_owner(&self) -> impl std::future::Future<Output = OwnerLookup> + Send;
    fn screenshot_permission(&self) -> impl std::future::Future<Output = Permission> + Send;
    /// Publish what the portal is up to for the app's notice.
    fn note(&self, state: crate::capture::types::PortalState) {
        crate::capture::state::note_portal_state(state);
    }
}

/// Why an unanswered request was given up, or that it keeps waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unanswered {
    /// The portal that took the request is gone or was replaced.
    PortalGone,
    /// Permission is granted, yet no answer: the portal stopped answering.
    Stalled,
    /// A permission prompt may be waiting for the player.
    MayBePrompt,
    /// Nothing could be read: keep waiting, change nothing.
    Unknown,
}

/// The verdict on a request still unanswered at a check. `taker` is the
/// unique name that sent the `Screenshot` method reply (the frontend that
/// took the request), once that reply came. `answered_before` is whether a
/// request of this process ever got a picture: where the permission store
/// has no entry and requests did answer, the frontend asks no permission,
/// so a hang cannot be a prompt.
pub(crate) async fn judge_unanswered<C: ShotChecks>(
    checks: &C,
    taker: Option<&str>,
    answered_before: bool,
) -> Unanswered {
    match checks.portal_owner().await {
        OwnerLookup::NoOwner => return Unanswered::PortalGone,
        OwnerLookup::Owner(owner) if taker.is_some_and(|taker| taker != owner) => {
            return Unanswered::PortalGone;
        }
        OwnerLookup::Owner(_) => {}
        OwnerLookup::Unknown => return Unanswered::Unknown,
    }
    match checks.screenshot_permission().await {
        Permission::Yes => Unanswered::Stalled,
        Permission::Absent if answered_before => Unanswered::Stalled,
        Permission::Absent | Permission::Other => Unanswered::MayBePrompt,
    }
}

/// The unique name that took a request (the `Screenshot` reply's sender).
pub(crate) type TakerSlot = std::sync::Mutex<Option<String>>;

/// Waits for the portal's answer to `request` (the outer `Err` is a request
/// the portal never took: no bus, a method error; the inner result is the
/// Response). Every [`SHOT_GIVE_UP`] without one, [`judge_unanswered`]
/// decides: a dead or stalled request is dropped (which closes it,
/// [`CloseOnDrop`]) and the grab fails; a possible permission prompt keeps
/// it waiting, with a notice. A real Response clears the notices; a
/// request that ends without one clears its own prompt notice.
///
/// [`CloseOnDrop`]: super::CloseOnDrop
pub(crate) async fn await_answer<F, C>(
    request: F,
    checks: &C,
    backoff: &std::sync::Mutex<ShotBackoff>,
    taker: &TakerSlot,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>>
where
    F: std::future::Future<
            Output = Result<Result<String, String>, Box<dyn std::error::Error + Send + Sync>>,
        >,
    C: ShotChecks,
{
    use futures_util::FutureExt as _;
    let request = crate::capture::flight::with_overdue_warnings(
        request,
        SHOT_OVERDUE_FIRST,
        SHOT_OVERDUE_EVERY,
        |waited| {
            tracing::warn!(
                waited_secs = waited.as_secs(),
                "portal screenshot request has not answered; screenshots wait for it"
            );
        },
    );
    tokio::pin!(request);
    let mut prompt_said = false;
    // Leaving without an answer: a prompt notice of this request goes.
    let clear_prompt = |prompt_said: bool| {
        if prompt_said {
            checks.note(crate::capture::types::PortalState::Answering);
        }
    };
    loop {
        let asked = tokio::time::timeout(SHOT_GIVE_UP, request.as_mut())
            .await
            .ok();
        let verdict = match asked {
            Some(asked) => Err(asked),
            None => {
                let answered_before = lock(backoff).answered_ok;
                let taker = lock(taker).clone();
                let verdict = judge_unanswered(checks, taker.as_deref(), answered_before).await;
                // An answer that came during the check is not dropped.
                match request.as_mut().now_or_never() {
                    Some(asked) => Err(asked),
                    None => Ok(verdict),
                }
            }
        };
        let verdict = match verdict {
            Err(Ok(answer)) => {
                // A Response arrived: the portal is there and answering.
                let mut state = lock(backoff);
                state.answered(answer.is_ok());
                drop(state);
                checks.note(crate::capture::types::PortalState::Answering);
                return Ok(answer?);
            }
            Err(Err(not_taken)) => {
                clear_prompt(prompt_said);
                return Err(not_taken);
            }
            Ok(verdict) => verdict,
        };
        match verdict {
            Unanswered::Unknown => {}
            Unanswered::MayBePrompt => {
                if !prompt_said {
                    prompt_said = true;
                    tracing::warn!(
                        "the screenshot portal has not answered and screenshot permission is not granted: a permission prompt may be waiting, possibly behind the game"
                    );
                }
                checks.note(crate::capture::types::PortalState::MayBePrompt);
            }
            Unanswered::PortalGone => {
                tracing::warn!(
                    "the screenshot portal went away during a request: it was given up and the next request goes to the new portal"
                );
                clear_prompt(prompt_said);
                return Err(format!("{SHOT_UNANSWERED} (the portal went away)").into());
            }
            Unanswered::Stalled => {
                // Set before the request is dropped and its Close runs.
                let pause = lock(backoff).gave_up(tokio::time::Instant::now());
                tracing::warn!(
                    waited_secs = SHOT_GIVE_UP.as_secs(),
                    pause_secs = pause.as_secs(),
                    unclosed_hold_secs = pause.max(SHOT_UNCLOSED_HOLD).as_secs(),
                    "{SHOT_UNANSWERED}: the request was closed and counts as a failed grab; ordinary polls make no portal request for pause_secs, or unclosed_hold_secs while the portal has not answered the Close"
                );
                checks.note(crate::capture::types::PortalState::Stalled);
                return Err(format!(
                    "{SHOT_UNANSWERED} (no answer in {} s)",
                    SHOT_GIVE_UP.as_secs()
                )
                .into());
            }
        }
    }
}

/// The real [`ShotChecks`], on the request's bus connection.
pub(super) struct BusChecks {
    pub(super) conn: ashpd::zbus::Connection,
}

impl ShotChecks for BusChecks {
    async fn portal_owner(&self) -> OwnerLookup {
        let reply = tokio::time::timeout(
            SHOT_CHECK_TIMEOUT,
            self.conn.call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(PORTAL_DESKTOP,),
            ),
        )
        .await;
        match reply {
            Ok(Ok(reply)) => reply
                .body()
                .deserialize::<String>()
                .map_or(OwnerLookup::Unknown, OwnerLookup::Owner),
            Ok(Err(ashpd::zbus::Error::MethodError(name, _, _)))
                if owner_error_is_gone(name.as_str()) =>
            {
                OwnerLookup::NoOwner
            }
            Ok(Err(_)) | Err(_) => OwnerLookup::Unknown,
        }
    }

    async fn screenshot_permission(&self) -> Permission {
        let reply = tokio::time::timeout(
            SHOT_CHECK_TIMEOUT,
            self.conn.call_method(
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "/org/freedesktop/impl/portal/PermissionStore",
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "Lookup",
                &("screenshot", "screenshot"),
            ),
        )
        .await;
        match reply {
            Ok(Ok(reply)) => reply
                .body()
                .deserialize::<(
                    std::collections::HashMap<String, Vec<String>>,
                    ashpd::zvariant::OwnedValue,
                )>()
                .map_or(Permission::Other, |(permissions, _)| {
                    screenshot_permission(&permissions)
                }),
            Ok(Err(ashpd::zbus::Error::MethodError(name, _, _)))
                if lookup_error_is_absent(name.as_str()) =>
            {
                Permission::Absent
            }
            Ok(Err(_)) | Err(_) => Permission::Other,
        }
    }
}

/// `GetNameOwner` failed because nothing owns the name.
pub fn owner_error_is_gone(error_name: &str) -> bool {
    error_name == "org.freedesktop.DBus.Error.NameHasNoOwner"
}

/// `Lookup` failed because the store has no `screenshot` entry ("No entry
/// for screenshot").
pub fn lookup_error_is_absent(error_name: &str) -> bool {
    error_name == "org.freedesktop.portal.Error.NotFound"
}

/// The permission store's `screenshot` entry for an unsandboxed app (app
/// id `""`, which the tracker is).
pub(crate) fn screenshot_permission(
    permissions: &std::collections::HashMap<String, Vec<String>>,
) -> Permission {
    match permissions.get("").and_then(|values| values.first()) {
        Some(value) if value == "yes" => Permission::Yes,
        Some(_) => Permission::Other,
        None => Permission::Absent,
    }
}

/// When an unanswered request is first warned about.
pub const SHOT_OVERDUE_FIRST: std::time::Duration = std::time::Duration::from_secs(20);

/// And how often after that, while it is not given up.
pub const SHOT_OVERDUE_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// How often an unanswered request is looked at, and so when one is first
/// given up (S5-CAP-1). At least the share dialog's 60 s timeout.
pub const SHOT_GIVE_UP: std::time::Duration = std::time::Duration::from_secs(60);

/// The error for a request given up.
pub const SHOT_UNANSWERED: &str = "the screenshot portal stopped answering";

/// How long one permission-store or name-owner read may take.
const SHOT_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

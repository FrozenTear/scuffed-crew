//! Pure policy for membership lifecycle and admin authority.
//!
//! Route handlers call these before mutating DB so rules stay in one place
//! and unit-testable without Surreal.
//!
//! ## HTTP status mapping
//!
//! - **403 Forbidden** — actor is not allowed (last-admin, hierarchy, self-action).
//! - **400 Bad Request** — authorized but invalid state (e.g. already has role).
//! - **409 Conflict** — concurrent CAS (application status); handled at the DB layer.

use axum::http::StatusCode;
use scuffed_db::{ApplicationStatus, ModerationActionType, OrgRole};

/// Policy denial with the HTTP status the route should return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDenial {
    BadRequest(&'static str),
    Forbidden(&'static str),
}

impl PolicyDenial {
    pub fn message(self) -> &'static str {
        match self {
            PolicyDenial::BadRequest(m) | PolicyDenial::Forbidden(m) => m,
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            PolicyDenial::BadRequest(_) => StatusCode::BAD_REQUEST,
            PolicyDenial::Forbidden(_) => StatusCode::FORBIDDEN,
        }
    }
}

// ─── Application status machine ─────────────────────────────────────────────

/// Valid officer-driven edges (same-status is not a transition).
pub fn is_valid_application_transition(from: ApplicationStatus, to: ApplicationStatus) -> bool {
    use ApplicationStatus::*;
    if from == to {
        return false;
    }
    matches!(
        (from, to),
        (Pending, Trial)
            | (Pending, Accepted)
            | (Pending, Rejected)
            | (Pending, Withdrawn)
            | (Trial, Accepted)
            | (Trial, Rejected)
            | (Trial, Withdrawn)
    )
}

/// Applicant may self-withdraw only from open pipeline states.
pub fn applicant_may_self_withdraw(from: ApplicationStatus) -> bool {
    matches!(from, ApplicationStatus::Pending | ApplicationStatus::Trial)
}

/// Statuses that block a second application while open.
pub fn application_blocks_resubmit(status: ApplicationStatus) -> bool {
    matches!(
        status,
        ApplicationStatus::Pending | ApplicationStatus::Trial
    )
}

/// Minutes left before a user whose latest application is closed (withdrawn,
/// rejected or accepted) may apply again, or `None` when the cooldown is
/// over. The cooldown is [`scuffed_types::APPLICATION_RESUBMIT_COOLDOWN_SECS`]
/// from the application's `updated_at`, which the status write sets.
///
/// Rounded up, so the answer is never "0 minutes". `now` is a parameter so
/// tests do not wait. A `updated_at` ahead of `now` (clock step) waits one
/// full cooldown, not longer.
pub fn application_resubmit_wait_minutes(
    last_updated_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<i64> {
    let cooldown = scuffed_types::APPLICATION_RESUBMIT_COOLDOWN_SECS;
    let elapsed = now.signed_duration_since(last_updated_at).num_seconds();
    let left = (cooldown - elapsed).min(cooldown);
    (left > 0).then(|| (left + 59) / 60)
}

/// True when the user has withdrawn as many applications inside the rolling
/// window as allowed ([`scuffed_types::MAX_WITHDRAWN_APPLICATIONS`] in the
/// last [`scuffed_types::WITHDRAWN_APPLICATION_WINDOW_DAYS`] days): no new
/// application, and no withdraw that would add another. `withdrawn_in_window`
/// counts only rows withdrawn after [`withdrawn_application_window_start`].
pub fn withdrawn_application_cap_reached(withdrawn_in_window: u64) -> bool {
    withdrawn_in_window >= scuffed_types::MAX_WITHDRAWN_APPLICATIONS
}

/// Start of the rolling withdrawn window at `now`. A row withdrawn after
/// this instant counts toward the cap; one withdrawn at or before it does not.
pub fn withdrawn_application_window_start(
    now: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    now - chrono::Duration::seconds(scuffed_types::WITHDRAWN_APPLICATION_WINDOW_SECS)
}

/// How long until a user at the withdrawn cap has a free slot again, as
/// text for the 429 ("3 days", "1 day", "5 hours", "1 minute").
///
/// `slot_withdrawn_at` is when the row that leaves the window next was
/// withdrawn: the oldest of the newest
/// [`scuffed_types::MAX_WITHDRAWN_APPLICATIONS`] in-window rows. The slot
/// opens one window after that.
///
/// Rounded up in the unit shown, so the wait is never understated and never
/// "0 days": a day or more is said in days, an hour or more in hours, less
/// in minutes (at least 1). `now` is a parameter so tests do not wait. A
/// `slot_withdrawn_at` ahead of `now` (clock step) waits one full window,
/// not longer.
pub fn withdrawn_application_cap_wait(
    slot_withdrawn_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let window = scuffed_types::WITHDRAWN_APPLICATION_WINDOW_SECS;
    let elapsed = now.signed_duration_since(slot_withdrawn_at).num_seconds();
    let left = (window - elapsed).clamp(1, window);
    let (count, unit) = if left >= DAY {
        ((left + DAY - 1) / DAY, "day")
    } else if left >= HOUR {
        ((left + HOUR - 1) / HOUR, "hour")
    } else {
        ((left + MINUTE - 1) / MINUTE, "minute")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
}

/// Role to assign when creating a member for this application outcome.
pub fn role_on_application_accept(from: ApplicationStatus) -> OrgRole {
    match from {
        // Coming off trial → full member; direct accept → recruit pipeline
        ApplicationStatus::Trial => OrgRole::Member,
        _ => OrgRole::Recruit,
    }
}

/// Whether this status should provision/ensure a member record.
pub fn application_status_ensures_member(status: ApplicationStatus) -> bool {
    matches!(
        status,
        ApplicationStatus::Trial | ApplicationStatus::Accepted
    )
}

/// Whether this status should deactivate an existing recruit (failed trial).
pub fn application_status_deactivates_member(status: ApplicationStatus) -> bool {
    matches!(
        status,
        ApplicationStatus::Rejected | ApplicationStatus::Withdrawn
    )
}

// ─── Role hierarchy / authority ─────────────────────────────────────────────

/// Target ranks an officer may moderate (strictly below officer).
pub fn officer_may_moderate(target: OrgRole) -> bool {
    matches!(target, OrgRole::Member | OrgRole::Recruit)
}

/// Whether `actor` may edit another member's profile or game account.
///
/// Self-edits are allowed by the route before this runs. Admins may edit any
/// other profile. Officers may edit only ranks [`officer_may_moderate`] allows
/// (member and recruit). Members and recruits may edit only their own profile.
/// `PUT` and `DELETE /api/members/{id}/game-accounts` use this same rule for
/// someone else.
/// A profile edit does not remove admin authority, so the last-admin guards on
/// role changes and `is_active` stay on those paths.
pub fn can_edit_other_profile(actor: OrgRole, target: OrgRole) -> Result<(), PolicyDenial> {
    match actor {
        OrgRole::Admin => Ok(()),
        OrgRole::Officer => {
            if officer_may_moderate(target) {
                Ok(())
            } else {
                Err(PolicyDenial::Forbidden(
                    "Officers cannot edit the profile of admins or other officers",
                ))
            }
        }
        OrgRole::Member | OrgRole::Recruit => {
            Err(PolicyDenial::Forbidden("Can only edit your own profile"))
        }
    }
}

/// Whether `actor` may moderate `target` (ban/suspend/warn/note).
pub fn can_moderate(
    actor: OrgRole,
    target: OrgRole,
    actor_id: &str,
    target_id: &str,
) -> Result<(), PolicyDenial> {
    if actor_id == target_id {
        return Err(PolicyDenial::Forbidden("Cannot moderate yourself"));
    }
    match actor {
        OrgRole::Admin => Ok(()),
        OrgRole::Officer => {
            if officer_may_moderate(target) {
                Ok(())
            } else {
                Err(PolicyDenial::Forbidden(
                    "Officers cannot moderate admins or other officers",
                ))
            }
        }
        OrgRole::Member | OrgRole::Recruit => {
            Err(PolicyDenial::Forbidden("Insufficient role to moderate"))
        }
    }
}

/// Whether moderation action type should kill all sessions for the target.
pub fn moderation_revokes_sessions(action: ModerationActionType) -> bool {
    matches!(
        action,
        ModerationActionType::Suspension | ModerationActionType::Ban
    )
}

/// Rules for changing a member's org role (admin-only path, pre-checked).
///
/// `actionable_admin_count` is active admins who are **not** suspended/banned,
/// and includes the target if they are currently an actionable admin.
///
/// `target_is_actionable_admin` is true when the target is currently in that
/// count (active admin, not suspended/banned). Demoting an already-suspended
/// admin does not reduce the actionable count and is allowed.
#[allow(clippy::too_many_arguments)] // pure policy: explicit fields beat a param struct here
pub fn can_change_role(
    actor_id: &str,
    target_id: &str,
    target_role: OrgRole,
    target_is_active: bool,
    new_role: OrgRole,
    actionable_admin_count: u64,
    target_is_actionable_admin: bool,
) -> Result<(), PolicyDenial> {
    if target_role == new_role {
        return Err(PolicyDenial::BadRequest("Member already has this role"));
    }

    // Demoting the last actionable admin locks the org out of admin tools.
    let demoting_actionable_admin = target_is_actionable_admin
        && target_is_active
        && target_role == OrgRole::Admin
        && new_role != OrgRole::Admin;
    if demoting_actionable_admin && actionable_admin_count <= 1 {
        return Err(PolicyDenial::Forbidden(
            "Cannot demote the last active admin",
        ));
    }

    // Optional self-demote of last admin already covered; self-promote is fine.
    let _ = actor_id;
    let _ = target_id;
    Ok(())
}

/// Who may set `is_active` on a target profile.
#[allow(clippy::too_many_arguments)] // pure policy: explicit fields beat a param struct here
pub fn can_set_is_active(
    actor_id: &str,
    actor_role: OrgRole,
    target_id: &str,
    target_role: OrgRole,
    target_is_active: bool,
    new_active: bool,
    actionable_admin_count: u64,
    target_is_actionable_admin: bool,
) -> Result<(), PolicyDenial> {
    // Never let someone flip their own active flag (lockout / self-reactivate bypass).
    if actor_id == target_id {
        return Err(PolicyDenial::Forbidden(
            "Cannot change your own active status",
        ));
    }

    if !actor_role.is_at_least(OrgRole::Officer) {
        return Err(PolicyDenial::Forbidden(
            "Only officers or admins can change active status",
        ));
    }

    // Officers only act on member/recruit
    if actor_role == OrgRole::Officer && !officer_may_moderate(target_role) {
        return Err(PolicyDenial::Forbidden(
            "Officers cannot deactivate admins or other officers",
        ));
    }

    // Deactivating the last actionable admin
    if target_is_actionable_admin
        && target_is_active
        && !new_active
        && target_role == OrgRole::Admin
        && actionable_admin_count <= 1
    {
        return Err(PolicyDenial::Forbidden(
            "Cannot deactivate the last active admin",
        ));
    }

    Ok(())
}

/// Whether deactivating a member (is_active=false) should revoke sessions.
pub fn deactivation_revokes_sessions(was_active: bool, new_active: bool) -> bool {
    was_active && !new_active
}

/// Ban/suspend of last actionable admin is forbidden.
///
/// `actionable_admin_count` must exclude already-suspended/banned admins so
/// suspending the penultimate admin cannot leave zero usable admins.
/// Already-suspended admins may still be banned (they do not count).
pub fn can_suspend_or_ban_admin(
    target_role: OrgRole,
    target_is_active: bool,
    action: ModerationActionType,
    actionable_admin_count: u64,
    target_is_actionable_admin: bool,
) -> Result<(), PolicyDenial> {
    if !moderation_revokes_sessions(action) {
        return Ok(());
    }
    if target_is_actionable_admin
        && target_is_active
        && target_role == OrgRole::Admin
        && actionable_admin_count <= 1
    {
        return Err(PolicyDenial::Forbidden(
            "Cannot suspend or ban the last active admin",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resubmit_cooldown_is_one_hour_from_the_last_status_write() {
        use chrono::{Duration, TimeZone, Utc};
        let closed = Utc.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap();
        let at =
            |secs: i64| application_resubmit_wait_minutes(closed, closed + Duration::seconds(secs));
        // Right after the close: the full hour.
        assert_eq!(at(0), Some(60));
        // Rounded up, so never "0 minutes".
        assert_eq!(at(1), Some(60));
        assert_eq!(at(60), Some(59));
        assert_eq!(at(3540), Some(1));
        assert_eq!(at(3599), Some(1));
        // The hour is over at exactly 3600 s.
        assert_eq!(at(3600), None);
        assert_eq!(at(3601), None);
        assert_eq!(at(86_400), None);
        // `updated_at` ahead of the clock waits one cooldown, not longer.
        assert_eq!(at(-7200), Some(60));
    }

    #[test]
    fn withdrawn_cap_is_reached_at_five_rows_in_the_window() {
        assert!(!withdrawn_application_cap_reached(0));
        assert!(!withdrawn_application_cap_reached(4));
        assert!(withdrawn_application_cap_reached(5));
        // Rows already over the cap stay refused while they are in the window.
        assert!(withdrawn_application_cap_reached(6));
    }

    #[test]
    fn withdrawn_window_starts_thirty_days_back() {
        use chrono::{Duration, TimeZone, Utc};
        let now = Utc.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap();
        assert_eq!(
            withdrawn_application_window_start(now),
            now - Duration::days(30)
        );
        assert_eq!(
            withdrawn_application_window_start(now),
            Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap()
        );
    }

    #[test]
    fn withdrawn_cap_wait_counts_down_from_thirty_days() {
        use chrono::{Duration, TimeZone, Utc};
        const DAY: i64 = 86_400;
        let withdrawn = Utc.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap();
        let at = |secs: i64| {
            withdrawn_application_cap_wait(withdrawn, withdrawn + Duration::seconds(secs))
        };
        // Right after the withdraw: the full window.
        assert_eq!(at(0), "30 days");
        // Rounded up, so the wait is never understated.
        assert_eq!(at(1), "30 days");
        assert_eq!(at(DAY), "29 days");
        assert_eq!(at(27 * DAY), "3 days");
        assert_eq!(at(28 * DAY), "2 days");
        assert_eq!(at(28 * DAY + 1), "2 days");
        // Exactly one day left is the singular.
        assert_eq!(at(29 * DAY), "1 day");
        // Under a day: hours, not "0 days" or "1 day".
        assert_eq!(at(29 * DAY + 1), "24 hours");
        assert_eq!(at(29 * DAY + 3600), "23 hours");
        assert_eq!(at(30 * DAY - 2 * 3600), "2 hours");
        assert_eq!(at(30 * DAY - 3601), "2 hours");
        assert_eq!(at(30 * DAY - 3600), "1 hour");
        // Under an hour: minutes.
        assert_eq!(at(30 * DAY - 3599), "60 minutes");
        assert_eq!(at(30 * DAY - 120), "2 minutes");
        assert_eq!(at(30 * DAY - 60), "1 minute");
        assert_eq!(at(30 * DAY - 1), "1 minute");
        // Never "0 minutes", even if asked at or past the end of the window.
        assert_eq!(at(30 * DAY), "1 minute");
        assert_eq!(at(31 * DAY), "1 minute");
        // `updated_at` ahead of the clock waits one window, not longer.
        assert_eq!(at(-7 * DAY), "30 days");
    }

    #[test]
    fn application_transitions() {
        assert!(is_valid_application_transition(
            ApplicationStatus::Pending,
            ApplicationStatus::Accepted
        ));
        assert!(is_valid_application_transition(
            ApplicationStatus::Pending,
            ApplicationStatus::Trial
        ));
        assert!(is_valid_application_transition(
            ApplicationStatus::Trial,
            ApplicationStatus::Accepted
        ));
        assert!(!is_valid_application_transition(
            ApplicationStatus::Accepted,
            ApplicationStatus::Rejected
        ));
        assert!(!is_valid_application_transition(
            ApplicationStatus::Rejected,
            ApplicationStatus::Pending
        ));
        assert!(!is_valid_application_transition(
            ApplicationStatus::Pending,
            ApplicationStatus::Pending
        ));
    }

    #[test]
    fn last_admin_demote_blocked() {
        let err = can_change_role("a1", "a1", OrgRole::Admin, true, OrgRole::Officer, 1, true);
        assert!(matches!(err, Err(PolicyDenial::Forbidden(_))));
        assert!(
            can_change_role("a1", "a2", OrgRole::Admin, true, OrgRole::Officer, 2, true).is_ok()
        );
        // Suspended (non-actionable) admin may be demoted even if count is 1
        assert!(
            can_change_role("a1", "a2", OrgRole::Admin, true, OrgRole::Officer, 1, false).is_ok()
        );
        let already = can_change_role("a1", "a2", OrgRole::Admin, true, OrgRole::Admin, 2, true);
        assert!(matches!(already, Err(PolicyDenial::BadRequest(_))));
    }

    #[test]
    fn profile_edit_rank_matches_moderation() {
        assert!(can_edit_other_profile(OrgRole::Officer, OrgRole::Member).is_ok());
        assert!(can_edit_other_profile(OrgRole::Officer, OrgRole::Recruit).is_ok());
        assert!(can_edit_other_profile(OrgRole::Admin, OrgRole::Officer).is_ok());
        assert!(can_edit_other_profile(OrgRole::Admin, OrgRole::Admin).is_ok());

        let officer_admin = can_edit_other_profile(OrgRole::Officer, OrgRole::Admin).unwrap_err();
        assert_eq!(officer_admin.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            officer_admin.message(),
            "Officers cannot edit the profile of admins or other officers"
        );
        let officer_peer = can_edit_other_profile(OrgRole::Officer, OrgRole::Officer).unwrap_err();
        assert_eq!(officer_peer.status(), StatusCode::FORBIDDEN);
        assert_eq!(officer_peer.message(), officer_admin.message());

        let member = can_edit_other_profile(OrgRole::Member, OrgRole::Recruit).unwrap_err();
        assert_eq!(member.status(), StatusCode::FORBIDDEN);
        assert_eq!(member.message(), "Can only edit your own profile");
        let recruit = can_edit_other_profile(OrgRole::Recruit, OrgRole::Member).unwrap_err();
        assert_eq!(recruit.message(), "Can only edit your own profile");
    }

    #[test]
    fn officer_cannot_moderate_officer() {
        assert!(matches!(
            can_moderate(OrgRole::Officer, OrgRole::Officer, "o1", "o2"),
            Err(PolicyDenial::Forbidden(_))
        ));
        assert!(can_moderate(OrgRole::Officer, OrgRole::Member, "o1", "m1").is_ok());
        assert!(can_moderate(OrgRole::Admin, OrgRole::Admin, "a1", "a2").is_ok());
        assert!(matches!(
            can_moderate(OrgRole::Admin, OrgRole::Admin, "a1", "a1"),
            Err(PolicyDenial::Forbidden(_))
        ));
    }

    #[test]
    fn last_admin_deactivate_blocked() {
        assert!(matches!(
            can_set_is_active(
                "a1",
                OrgRole::Admin,
                "a2",
                OrgRole::Admin,
                true,
                false,
                1,
                true
            ),
            Err(PolicyDenial::Forbidden(_))
        ));
        assert!(
            can_set_is_active(
                "a1",
                OrgRole::Admin,
                "a2",
                OrgRole::Admin,
                true,
                false,
                2,
                true
            )
            .is_ok()
        );
        assert!(matches!(
            can_set_is_active(
                "m1",
                OrgRole::Member,
                "m2",
                OrgRole::Member,
                true,
                false,
                1,
                false
            ),
            Err(PolicyDenial::Forbidden(_))
        ));
        assert!(matches!(
            can_set_is_active(
                "a1",
                OrgRole::Admin,
                "a1",
                OrgRole::Admin,
                true,
                false,
                2,
                true
            ),
            Err(PolicyDenial::Forbidden(_))
        ));
    }

    #[test]
    fn accept_role_from_trial_is_member() {
        assert_eq!(
            role_on_application_accept(ApplicationStatus::Trial),
            OrgRole::Member
        );
        assert_eq!(
            role_on_application_accept(ApplicationStatus::Pending),
            OrgRole::Recruit
        );
    }

    #[test]
    fn ban_last_admin_blocked() {
        assert!(matches!(
            can_suspend_or_ban_admin(OrgRole::Admin, true, ModerationActionType::Ban, 1, true),
            Err(PolicyDenial::Forbidden(_))
        ));
        assert!(
            can_suspend_or_ban_admin(OrgRole::Admin, true, ModerationActionType::Ban, 2, true)
                .is_ok()
        );
        assert!(
            can_suspend_or_ban_admin(OrgRole::Member, true, ModerationActionType::Ban, 1, false)
                .is_ok()
        );
        // Already-suspended admin may be banned (does not count as actionable)
        assert!(
            can_suspend_or_ban_admin(OrgRole::Admin, true, ModerationActionType::Ban, 1, false)
                .is_ok()
        );
    }

    #[test]
    fn self_withdraw_only_open_pipeline() {
        assert!(applicant_may_self_withdraw(ApplicationStatus::Pending));
        assert!(applicant_may_self_withdraw(ApplicationStatus::Trial));
        assert!(!applicant_may_self_withdraw(ApplicationStatus::Accepted));
        assert!(!applicant_may_self_withdraw(ApplicationStatus::Rejected));
        assert!(!applicant_may_self_withdraw(ApplicationStatus::Withdrawn));
    }
}

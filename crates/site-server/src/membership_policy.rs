//! Pure policy for membership lifecycle and admin authority.
//!
//! Route handlers call these before mutating DB so rules stay in one place
//! and unit-testable without Surreal.

use scuffed_db::{ApplicationStatus, ModerationActionType, OrgRole};

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

/// Statuses that block a second application while open.
pub fn application_blocks_resubmit(status: ApplicationStatus) -> bool {
    matches!(
        status,
        ApplicationStatus::Pending | ApplicationStatus::Trial
    )
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

/// Whether `actor` may moderate `target` (ban/suspend/warn/note).
pub fn can_moderate(actor: OrgRole, target: OrgRole, actor_id: &str, target_id: &str) -> Result<(), &'static str> {
    if actor_id == target_id {
        return Err("Cannot moderate yourself");
    }
    match actor {
        OrgRole::Admin => Ok(()),
        OrgRole::Officer => {
            if officer_may_moderate(target) {
                Ok(())
            } else {
                Err("Officers cannot moderate admins or other officers")
            }
        }
        OrgRole::Member | OrgRole::Recruit => Err("Insufficient role to moderate"),
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
pub fn can_change_role(
    actor_id: &str,
    target_id: &str,
    target_role: OrgRole,
    target_is_active: bool,
    new_role: OrgRole,
    actionable_admin_count: u64,
    target_is_actionable_admin: bool,
) -> Result<(), &'static str> {
    if target_role == new_role {
        return Err("Member already has this role");
    }

    // Demoting the last actionable admin locks the org out of admin tools.
    let demoting_actionable_admin =
        target_is_actionable_admin && target_is_active && target_role == OrgRole::Admin && new_role != OrgRole::Admin;
    if demoting_actionable_admin && actionable_admin_count <= 1 {
        return Err("Cannot demote the last active admin");
    }

    // Optional self-demote of last admin already covered; self-promote is fine.
    let _ = actor_id;
    let _ = target_id;
    Ok(())
}

/// Who may set `is_active` on a target profile.
pub fn can_set_is_active(
    actor_id: &str,
    actor_role: OrgRole,
    target_id: &str,
    target_role: OrgRole,
    target_is_active: bool,
    new_active: bool,
    actionable_admin_count: u64,
    target_is_actionable_admin: bool,
) -> Result<(), &'static str> {
    // Never let someone flip their own active flag (lockout / self-reactivate bypass).
    if actor_id == target_id {
        return Err("Cannot change your own active status");
    }

    if !actor_role.is_at_least(OrgRole::Officer) {
        return Err("Only officers or admins can change active status");
    }

    // Officers only act on member/recruit
    if actor_role == OrgRole::Officer && !officer_may_moderate(target_role) {
        return Err("Officers cannot deactivate admins or other officers");
    }

    // Deactivating the last actionable admin
    if target_is_actionable_admin
        && target_is_active
        && !new_active
        && target_role == OrgRole::Admin
        && actionable_admin_count <= 1
    {
        return Err("Cannot deactivate the last active admin");
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
) -> Result<(), &'static str> {
    if !moderation_revokes_sessions(action) {
        return Ok(());
    }
    if target_is_actionable_admin
        && target_is_active
        && target_role == OrgRole::Admin
        && actionable_admin_count <= 1
    {
        return Err("Cannot suspend or ban the last active admin");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(err.is_err());
        assert!(can_change_role("a1", "a2", OrgRole::Admin, true, OrgRole::Officer, 2, true).is_ok());
        // Suspended (non-actionable) admin may be demoted even if count is 1
        assert!(can_change_role("a1", "a2", OrgRole::Admin, true, OrgRole::Officer, 1, false).is_ok());
    }

    #[test]
    fn officer_cannot_moderate_officer() {
        assert!(can_moderate(OrgRole::Officer, OrgRole::Officer, "o1", "o2").is_err());
        assert!(can_moderate(OrgRole::Officer, OrgRole::Member, "o1", "m1").is_ok());
        assert!(can_moderate(OrgRole::Admin, OrgRole::Admin, "a1", "a2").is_ok());
        assert!(can_moderate(OrgRole::Admin, OrgRole::Admin, "a1", "a1").is_err());
    }

    #[test]
    fn last_admin_deactivate_blocked() {
        assert!(can_set_is_active(
            "a1",
            OrgRole::Admin,
            "a2",
            OrgRole::Admin,
            true,
            false,
            1,
            true
        )
        .is_err());
        assert!(can_set_is_active(
            "a1",
            OrgRole::Admin,
            "a2",
            OrgRole::Admin,
            true,
            false,
            2,
            true
        )
        .is_ok());
        assert!(can_set_is_active(
            "m1",
            OrgRole::Member,
            "m2",
            OrgRole::Member,
            true,
            false,
            1,
            false
        )
        .is_err());
        assert!(can_set_is_active(
            "a1",
            OrgRole::Admin,
            "a1",
            OrgRole::Admin,
            true,
            false,
            2,
            true
        )
        .is_err());
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
        assert!(can_suspend_or_ban_admin(
            OrgRole::Admin,
            true,
            ModerationActionType::Ban,
            1,
            true
        )
        .is_err());
        assert!(can_suspend_or_ban_admin(
            OrgRole::Admin,
            true,
            ModerationActionType::Ban,
            2,
            true
        )
        .is_ok());
        assert!(can_suspend_or_ban_admin(
            OrgRole::Member,
            true,
            ModerationActionType::Ban,
            1,
            false
        )
        .is_ok());
        // Already-suspended admin may be banned (does not count as actionable)
        assert!(can_suspend_or_ban_admin(
            OrgRole::Admin,
            true,
            ModerationActionType::Ban,
            1,
            false
        )
        .is_ok());
    }
}

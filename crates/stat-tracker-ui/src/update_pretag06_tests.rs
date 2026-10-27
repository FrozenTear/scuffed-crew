//! Updater tests for the 0.6 prerelease line: an alpha install is never
//! offered 0.5.x, the channel follows the installed version, and a package
//! manager owner gets the package hint.

use crate::restart_prompt::RestartNeed;
use crate::update::*;

fn rel(tag: &str, prerelease: bool) -> serde_json::Value {
    serde_json::json!({
        "tag_name": tag, "draft": false, "prerelease": prerelease,
        "html_url": "https://example.invalid", "body": tag,
    })
}

fn world() -> Vec<serde_json::Value> {
    vec![
        rel("stat-tracker-v0.6.0-alpha.1", true),
        rel("stat-tracker-v0.5.1-rc.1", true),
        rel("stat-tracker-v0.5.1", false),
        rel("stat-tracker-v0.5.0", false),
        rel("stat-tracker-v0.5.0-rc.7", true),
        rel("stat-tracker-v0.5.0-alpha.12", true),
        rel("stat-tracker-v0.4.24", false),
        rel("site-2026-10-27", false),
    ]
}

#[test]
fn pretag06_alpha_install_never_offered_05x() {
    let r = world();
    assert!(!is_newer("0.5.0", "0.6.0-alpha.1"));
    assert!(!is_newer("0.5.1", "0.6.0-alpha.1"));
    assert!(!is_newer("0.5.99", "0.6.0-alpha.1"));
    assert!(!is_newer("0.5.1-rc.1", "0.6.0-alpha.1"));
    assert!(is_newer("0.6.0-alpha.1", "0.5.1"));
    assert_eq!(
        effective_update_channel("", "0.6.0-alpha.1"),
        UpdateChannel::Prerelease
    );
    for cur in ["0.6.0-alpha.1", "0.6.0-alpha.2"] {
        for ch in [UpdateChannel::Prerelease, UpdateChannel::Stable] {
            assert!(
                select_newer_release_on(ch, cur, &r).is_none(),
                "{cur} {ch:?}"
            );
        }
        for stored in ["", "stable", "prerelease", "garbage"] {
            let ch = effective_update_channel(stored, cur);
            assert!(
                select_newer_release_on(ch, cur, &r).is_none(),
                "{cur} {stored:?}"
            );
        }
    }
    // Next alpha is still offered.
    let mut next = r.clone();
    next.push(rel("stat-tracker-v0.6.0-alpha.2", true));
    let ch = effective_update_channel("", "0.6.0-alpha.1");
    let info = select_newer_release_on(ch, "0.6.0-alpha.1", &next).unwrap();
    assert_eq!(info.latest, "0.6.0-alpha.2");
    assert_eq!(
        info.release_bodies
            .iter()
            .map(|b| b.version.as_str())
            .collect::<Vec<_>>(),
        ["0.6.0-alpha.2"]
    );
}

#[test]
fn pretag06_stable_05_install_not_pushed_to_alpha() {
    let r = world();
    // 0.5.0 stable, channel key unset or "stable": only 0.5.1 stable.
    for stored in ["", "stable"] {
        let ch = effective_update_channel(stored, "0.5.0");
        assert_eq!(ch, UpdateChannel::Stable);
        let info = select_newer_release_on(ch, "0.5.0", &r).unwrap();
        assert_eq!(info.latest, "0.5.1", "{stored:?}");
        assert!(info.release_bodies.iter().all(|b| !b.version.contains('-')));
    }
    // Latest stable is 0.5.1 and the install is on it: nothing.
    assert!(select_newer_release_on(effective_update_channel("", "0.5.1"), "0.5.1", &r).is_none());
    // Unflagged alpha on GitHub still skipped on stable.
    let unflagged = vec![
        rel("stat-tracker-v0.6.0-alpha.1", false),
        rel("stat-tracker-v0.5.0", false),
    ];
    assert!(select_newer_release("0.5.0", &unflagged).is_none());
    // Opted in: offered the alpha.
    let ch = effective_update_channel("prerelease", "0.5.0");
    let info = select_newer_release_on(ch, "0.5.0", &r).unwrap();
    assert_eq!(info.latest, "0.6.0-alpha.1");
}

#[test]
fn pretag06_aur_install_gets_package_hint_on_alpha() {
    use crate::uninstall::{PackageManager, PackageOwner};
    let aur = PackageOwner {
        manager: PackageManager::Pacman,
        package: "scuffed-stat-tracker".into(),
    };
    let plan = evaluate_plan_for("0.6.0-alpha.2", PackageProbe::Done(Some(&aur)));
    assert!(!plan.can_run());
    let info = UpdateInfo {
        latest: "0.6.0-alpha.2".into(),
        current: "0.6.0-alpha.1".into(),
        url: String::new(),
        release_bodies: Vec::new(),
    };
    let m = banner_model(
        &info,
        &UpdateProgress::Idle,
        &plan,
        RestartNeed::Unknown,
        PackageProbe::Done(Some(&aur)),
    );
    assert!(m.command.is_none() && m.primary.is_none());
    let line = m.package_line.unwrap();
    assert!(line.contains("paru -Syu"));
}

#[test]
fn pretag06_alpha_bin_package_owner_is_a_package_manager_install() {
    use crate::uninstall::{PackageManager, probe_package_owner_with};
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let pacman = dir.path().join("pacman");
    std::fs::write(
        &pacman,
        "#!/bin/sh\nif [ \"$1\" = -Qo ]; then echo \"$2 is owned by scuffed-stat-tracker-alpha-bin 0.6.0alpha1-1\"; exit 0; fi\nexit 1\n",
    )
    .unwrap();
    let dpkg = dir.path().join("dpkg");
    std::fs::write(&dpkg, "#!/bin/sh\nexit 1\n").unwrap();
    for f in [&pacman, &dpkg] {
        std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let owner = probe_package_owner_with(
        std::path::Path::new("/usr/bin/stat-tracker-gui"),
        &pacman,
        &dpkg,
    )
    .expect("the alpha package owns the binary");
    assert_eq!(owner.manager, PackageManager::Pacman);
    assert_eq!(owner.package, "scuffed-stat-tracker-alpha-bin");

    // A package-owned install is never Ready and shows the package-manager hint.
    let plan = evaluate_plan_for("0.6.0-alpha.2", PackageProbe::Done(Some(&owner)));
    let UpdatePlan::Blocked { reason, hint } = &plan else {
        panic!("package plan must be blocked: {plan:?}");
    };
    assert!(
        reason.contains("scuffed-stat-tracker-alpha-bin"),
        "{reason}"
    );
    assert_eq!(*hint, package_update_line(&owner));
    assert!(!plan.can_run());
    let info = UpdateInfo {
        latest: "0.6.0-alpha.2".into(),
        current: "0.6.0-alpha.1".into(),
        url: String::new(),
        release_bodies: Vec::new(),
    };
    let m = banner_model(
        &info,
        &UpdateProgress::Idle,
        &plan,
        RestartNeed::Unknown,
        PackageProbe::Done(Some(&owner)),
    );
    assert!(m.command.is_none() && m.primary.is_none());
    let line = m.package_line.expect("package hint shown");
    assert!(line.contains("paru -Syu"), "{line}");
}

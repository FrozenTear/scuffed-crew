//! Tests of the GUI to daemon command files.

use super::*;

#[test]
fn command_queue_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = StoreCommand::SetOutcome {
        session_id: "s1".into(),
        outcome: "defeat".into(),
    };
    let b = StoreCommand::DeleteSession {
        session_id: "s2".into(),
    };
    queue_command(dir.path(), &a).unwrap();
    queue_command(dir.path(), &b).unwrap();
    let cmds = read_commands(dir.path());
    assert_eq!(cmds.len(), 2);
    assert!(
        matches!(&cmds[0].1, StoreCommand::SetOutcome { session_id, outcome }
        if session_id == "s1" && outcome == "defeat")
    );
    assert!(
        matches!(&cmds[1].1, StoreCommand::DeleteSession { session_id }
        if session_id == "s2")
    );
    // Two-phase: still queued until the caller removes the files...
    assert_eq!(read_commands(dir.path()).len(), 2);
    // ...and gone once each applied command's file is removed.
    for (path, _) in &cmds {
        remove_command_file(path);
    }
    assert!(read_commands(dir.path()).is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_mode = std::fs::metadata(dir.path().join("commands"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "command queue dir must be owner-only");
        // Files are removed after the roundtrip; write one more and check it.
        queue_command(
            dir.path(),
            &StoreCommand::DeleteSession {
                session_id: "perm".into(),
            },
        )
        .unwrap();
        let queued = read_commands(dir.path());
        assert_eq!(queued.len(), 1);
        let file_mode = std::fs::metadata(&queued[0].0)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600, "command queue files must be owner-only");
    }
}

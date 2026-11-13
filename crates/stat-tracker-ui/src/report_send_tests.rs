//! Send outcomes, the create URL, and saving: the upload and the Save
//! report file of [`super`].

use super::*;
use chrono::TimeZone;

#[test]
fn send_outcomes_follow_section_7() {
    let ok = outcome_for(
        201,
        None,
        r#"{"id":"abc","training":false,"expires_at":"2026-11-11T20:00:00Z"}"#,
        Some("https://site/reports".into()),
    );
    assert_eq!(
        ok,
        SendOutcome::Sent {
            id: "abc".into(),
            expires_at: Some(Utc.with_ymd_and_hms(2026, 11, 11, 20, 0, 0).unwrap()),
            reports_url: "https://site/reports".into(),
        }
    );
    let training = outcome_for(
        201,
        None,
        r#"{"id":"x","training":true,"expires_at":null}"#,
        None,
    );
    assert!(matches!(
        training,
        SendOutcome::Sent {
            expires_at: None,
            ..
        }
    ));

    let SendOutcome::Failed(m) = outcome_for(401, None, "", None) else {
        panic!()
    };
    assert!(m.contains("no longer valid") && m.contains("Link"));
    let SendOutcome::Failed(m) = outcome_for(429, Some(3600), "", None) else {
        panic!()
    };
    assert!(
        m.contains("daily report limit") && m.contains("60 minutes"),
        "{m}"
    );
    let SendOutcome::Failed(m) =
        outcome_for(429, None, r#"{"error":"Daily report limit reached"}"#, None)
    else {
        panic!()
    };
    assert!(
        m.starts_with("Daily report limit reached.") && m.contains("send it tomorrow"),
        "{m}"
    );
    let SendOutcome::Failed(m) = outcome_for(429, None, "", None) else {
        panic!()
    };
    assert!(!m.contains("Try again in"), "{m}");
    let SendOutcome::Failed(m) = outcome_for(503, None, r#"{"error":"reports_disabled"}"#, None)
    else {
        panic!()
    };
    assert!(m.starts_with("Reports are off right now"), "{m}");
    let SendOutcome::Failed(m) = outcome_for(500, None, "", None) else {
        panic!()
    };
    assert!(m.contains("was not sent"), "{m}");
    // A 400 is the site not taking the contents: no "Try again".
    let SendOutcome::Failed(m) = outcome_for(400, None, r#"{"error":"bad"}"#, None) else {
        panic!()
    };
    assert_eq!(m, "The site did not accept this report. Use Save report.");
    assert!(!m.contains("Try again"), "{m}");
    assert!(
        !m.contains("version"),
        "not every 400 is the site's age: {m}"
    );
    // A 201 without a readable body is not shown as sent.
    assert!(matches!(
        outcome_for(201, None, "nope", None),
        SendOutcome::Failed(_)
    ));
}

#[test]
fn create_url_uses_the_linked_site_only() {
    assert_eq!(
        create_url("https://scuffed.example/").unwrap(),
        "https://scuffed.example/api/stat-reports"
    );
    assert!(create_url("not a url").is_err());
}

#[test]
fn download_dir_reads_user_dirs_and_falls_back() {
    let home = Path::new("/home/qx");
    assert_eq!(
        parse_download_dir("# c\nXDG_DOWNLOAD_DIR=\"$HOME/Hent\"\n", home),
        Some(PathBuf::from("/home/qx/Hent"))
    );
    assert_eq!(
        parse_download_dir("XDG_DOWNLOAD_DIR=\"/data/dl\"", home),
        Some(PathBuf::from("/data/dl"))
    );
    // xdg-user-dirs writes "$HOME/" when the folder is disabled.
    assert_eq!(
        parse_download_dir("XDG_DOWNLOAD_DIR=\"$HOME/\"", home),
        None
    );
    assert_eq!(parse_download_dir("XDG_DOWNLOAD_DIR=\"rel\"", home), None);
    assert_eq!(parse_download_dir("XDG_MUSIC_DIR=\"$HOME/M\"", home), None);

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(home.join(".config")).unwrap();
    assert_eq!(download_dir(&home, None), home.join("Downloads"));
    fs::write(
        home.join(".config/user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Nedlastinger\"\n",
    )
    .unwrap();
    assert_eq!(download_dir(&home, None), home.join("Nedlastinger"));
    let xdg = tmp.path().join("xdg");
    fs::create_dir_all(&xdg).unwrap();
    fs::write(xdg.join("user-dirs.dirs"), "XDG_DOWNLOAD_DIR=\"/srv/dl\"\n").unwrap();
    assert_eq!(download_dir(&home, Some(&xdg)), PathBuf::from("/srv/dl"));
}

#[test]
fn save_never_overwrites_and_is_private() {
    let now = Utc.with_ymd_and_hms(2026, 10, 12, 18, 5, 9).unwrap();
    let name = report_file_name(now);
    assert_eq!(name, "scuffed-report-20261012T180509Z.zip");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("Downloads");
    let a = save_zip(&dir, &name, b"one").unwrap();
    let b = save_zip(&dir, &name, b"two").unwrap();
    let c = save_zip(&dir, &name, b"three").unwrap();
    assert_eq!(a, dir.join("scuffed-report-20261012T180509Z.zip"));
    assert_eq!(b, dir.join("scuffed-report-20261012T180509Z-2.zip"));
    assert_eq!(c, dir.join("scuffed-report-20261012T180509Z-3.zip"));
    assert_eq!(fs::read(&a).unwrap(), b"one");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&a).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

/// A site that redirects the upload to another port is refused by
/// `token_safe_redirects`, and the failure names the redirect instead
/// of "Could not reach the site". Nothing reaches the target.
#[tokio::test]
async fn send_names_a_refused_off_site_redirect() {
    use std::io::{Read as _, Write as _};
    let elsewhere = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    elsewhere.set_nonblocking(true).expect("nonblocking");
    let target = format!(
        "http://127.0.0.1:{}/api/reports",
        elsewhere.local_addr().expect("addr").port()
    );
    let origin = target.trim_end_matches("/api/reports").to_string();
    let site = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let site_url = format!(
        "http://127.0.0.1:{}",
        site.local_addr().expect("addr").port()
    );
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = site.accept() else {
            return;
        };
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let resp = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(resp.as_bytes());
    });
    let zip = Arc::new(ReportZip {
        bytes: vec![1, 2, 3],
        manifest_json: "{}".into(),
        files: Vec::new(),
        log_text: String::new(),
        images: Vec::new(),
        left_out: Vec::new(),
    });

    let outcome = send(site_url, "tok-must-stay-home".into(), zip).await;
    assert_eq!(
        outcome,
        SendOutcome::Failed(format!(
            "The report was not sent. The site redirected to {origin}. \
             If that is your site's new address, set the Website URL in Settings to it."
        ))
    );
    let SendOutcome::Failed(text) = &outcome else {
        unreachable!()
    };
    assert!(!text.contains("/api/reports"), "{text}");
    assert!(
        elsewhere.accept().is_err(),
        "nothing may reach the redirect target"
    );
}

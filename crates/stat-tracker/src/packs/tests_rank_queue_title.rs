//! The rank-v1 and queue-title-v1 packs: a real unpack and sync of tars
//! built here from tiny synthetic `templates.json` files, the readers
//! loading what was installed, a folder placed by hand or edited since the
//! last install (kept under `templates-kept/`), a pack its reader refuses,
//! a keep that cannot happen, and the cases where no pack arrives.

use super::layout::{QUEUE_TITLE_PACK_DIR, QUEUE_TITLE_PACK_NAME, RANK_PACK_DIR, RANK_PACK_NAME};
use super::*;
use crate::shadow::{queue_title, rank};

/// A rank-v1 `templates.json` the reader accepts: the gate template and
/// one banner class, flat vectors of the size each family compares at.
fn rank_json() -> Vec<u8> {
    let class = |class: &str, family: rank::Family| {
        let (w, h, _) = family.shape();
        serde_json::json!({ "class": class, "aspect": 1.0, "n": 1, "vector": vec![0.5_f32; w * h] })
    };
    serde_json::to_vec(&serde_json::json!({
        "recognizer": rank::RECOGNIZER_ID,
        "families": {
            "gate": [class(rank::GATE_CLASS, rank::Family::Gate)],
            "banner": [class("COMPETITIVE: ROLE QUEUE", rank::Family::Banner)],
        },
    }))
    .expect("json")
}

/// A queue-title-v1 `templates.json` the reader accepts: two titles.
fn queue_title_json() -> Vec<u8> {
    let class = |class: &str| serde_json::json!({ "class": class, "n": 1, "vector": vec![0.25_f32; queue_title::TW * queue_title::TH] });
    serde_json::to_vec(&serde_json::json!({
        "recognizer": queue_title::RECOGNIZER_ID,
        "width": queue_title::TW,
        "height": queue_title::TH,
        "classes": [class("OPEN QUEUE"), class("ROLE QUEUE")],
    }))
    .expect("json")
}

/// The one file of each pack, as [`super::sample_files`] serves it too.
pub(super) fn pack_files(name: &str) -> Vec<(String, Vec<u8>)> {
    let data = match name {
        RANK_PACK_NAME => rank_json(),
        QUEUE_TITLE_PACK_NAME => queue_title_json(),
        other => panic!("no templates for {other}"),
    };
    vec![("templates.json".to_string(), data)]
}

/// A site that lists and serves only `names`, version `version`.
fn serve_only(names: &[&str], version: &str) -> Served {
    let mut list = Vec::new();
    let mut files = BTreeMap::new();
    for name in names {
        let bytes = build_pack(name, version, &pack_files(name));
        list.push(entry_json(name, version, &bytes));
        files.insert(name.to_string(), (200, bytes));
    }
    serve(list, files)
}

fn both_load(data_dir: &Path) -> bool {
    rank::Reader::load(data_dir).is_ok() && queue_title::Reader::load(data_dir).is_ok()
}

#[test]
fn both_packs_are_optional_and_land_in_the_folders_the_readers_load_from() {
    assert_eq!(RANK_PACK_NAME, "rank-v1.tar");
    assert_eq!(QUEUE_TITLE_PACK_NAME, "queue-title-v1.tar");
    assert_eq!(RANK_PACK_DIR, rank::TEMPLATE_DIR);
    assert_eq!(QUEUE_TITLE_PACK_DIR, queue_title::TEMPLATE_DIR);
    let data = Path::new("/data");
    for (name, dir) in [
        (RANK_PACK_NAME, rank::TEMPLATE_DIR),
        (QUEUE_TITLE_PACK_NAME, queue_title::TEMPLATE_DIR),
    ] {
        let spec = pack_spec(name).expect(name);
        assert!(!spec.required, "{name}");
        assert_eq!(spec.layout, PackLayout::TemplatesJson, "{name}");
        assert_eq!(spec.dir_in(data), data.join(dir), "{name}");
    }
    // The two come last, after every pack an older tracker asks for.
    let names: Vec<&str> = PACKS.iter().map(|spec| spec.name).collect();
    assert_eq!(
        names[names.len() - 2..],
        [RANK_PACK_NAME, QUEUE_TITLE_PACK_NAME]
    );
    assert_eq!(rank::templates::PACK_FILE, "templates.json");
    assert_eq!(queue_title::PACK_FILE, "templates.json");
}

#[test]
fn the_layout_allows_templates_json_and_nothing_else() {
    let layout = PackLayout::TemplatesJson;
    for raw in ["templates.json", "manifest.json"] {
        assert_eq!(layout.file_path(raw).as_deref(), Ok(raw), "{raw}");
    }
    for raw in [
        "templates.json.bak-roles-20261114",
        "Templates.json",
        "templates.json5",
        "other.json",
        "sub/templates.json",
        "../templates.json",
        "/templates.json",
        "templates.png",
    ] {
        assert_eq!(layout.file_path(raw), Err(PACK_UNSAFE), "{raw}");
    }
    assert!(!layout.allows_dir("special"));
    assert!(!layout.keeps_manifest());
}

#[tokio::test]
async fn real_packs_unpack_into_templates_and_both_readers_load_them() {
    let served = serve_only(&[RANK_PACK_NAME, QUEUE_TITLE_PACK_NAME], "full-20261114");
    let dir = tempfile::tempdir().expect("tempdir");
    let report = sync(&served, dir.path()).await;
    for (name, sub) in [
        (RANK_PACK_NAME, RANK_PACK_DIR),
        (QUEUE_TITLE_PACK_NAME, QUEUE_TITLE_PACK_DIR),
    ] {
        assert_eq!(outcome(&report, name), &PackSync::Installed, "{name}");
        let live = dir.path().join(sub);
        assert_eq!(
            std::fs::read(live.join("templates.json")).unwrap_or_default(),
            pack_files(name)[0].1,
            "{name}"
        );
        assert!(!live.join("manifest.json").exists(), "{name}");
        assert_eq!(stored_versions(dir.path())[name], "full-20261114");
    }
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
    let rank_set = rank::TemplateSet::load(dir.path()).expect("rank-v1 loads");
    assert_eq!(
        rank_set,
        serde_json::from_slice::<rank::TemplateSet>(&rank_json()).expect("json")
    );
    assert!(
        both_load(dir.path()),
        "the readers load what the sync installed"
    );

    // A second sync with the same list leaves them as they are.
    let again = sync(&served, dir.path()).await;
    assert_eq!(outcome(&again, RANK_PACK_NAME), &PackSync::Current);
    assert_eq!(outcome(&again, QUEUE_TITLE_PACK_NAME), &PackSync::Current);
}

/// The folders under `templates-kept/`, by name.
fn kept_folders(data_dir: &Path) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = std::fs::read_dir(data_dir.join(layout::KEPT_DIR))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    kept.sort();
    kept
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A rank-v1 folder placed by hand: its `templates.json` and a backup.
fn plant_hand_rank(data_dir: &Path) -> PathBuf {
    let hand = data_dir.join(RANK_PACK_DIR);
    std::fs::create_dir_all(&hand).unwrap();
    std::fs::write(hand.join("templates.json"), b"{\"hand\":true}").unwrap();
    std::fs::write(hand.join("templates.json.bak-roles-20261114"), b"old").unwrap();
    hand
}

#[tokio::test]
async fn a_folder_placed_by_hand_is_replaced_whole_by_the_first_sync() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hand = plant_hand_rank(dir.path());
    // No version is recorded for a folder placed by hand, so it is fetched.
    let served = serve_only(&[RANK_PACK_NAME], "full-20261114");
    let report = sync(&served, dir.path()).await;
    assert_eq!(outcome(&report, RANK_PACK_NAME), &PackSync::Installed);
    assert_eq!(
        file_names(&hand),
        ["templates.json"],
        "the extra file goes with the old folder"
    );
    assert_eq!(
        std::fs::read(hand.join("templates.json")).unwrap(),
        rank_json()
    );
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
    // The old folder is kept whole beside `templates/`, not deleted.
    let kept = kept_folders(dir.path());
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(
        kept[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("rank-v1-"),
        "{kept:?}"
    );
    assert_eq!(
        file_names(&kept[0]),
        ["templates.json", "templates.json.bak-roles-20261114"]
    );
    assert_eq!(
        std::fs::read(kept[0].join("templates.json")).unwrap(),
        b"{\"hand\":true}"
    );
    assert_eq!(
        std::fs::read(kept[0].join("templates.json.bak-roles-20261114")).unwrap(),
        b"old"
    );

    // Once a version is recorded, the next new version swaps as before and
    // keeps nothing more.
    let served = serve_only(&[RANK_PACK_NAME], "full-20261115");
    let report = sync(&served, dir.path()).await;
    assert_eq!(outcome(&report, RANK_PACK_NAME), &PackSync::Installed);
    assert_eq!(kept_folders(dir.path()), kept);
}

/// A tar the layout allows whose `templates.json` the reader refuses is
/// refused before the swap: the folder placed by hand stays where it was,
/// nothing is kept or staged, and no version is recorded.
#[tokio::test]
async fn a_pack_its_reader_cannot_load_is_refused_and_the_hand_folder_stays() {
    for (name, sub) in [
        (RANK_PACK_NAME, RANK_PACK_DIR),
        (QUEUE_TITLE_PACK_NAME, QUEUE_TITLE_PACK_DIR),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let hand = dir.path().join(sub);
        std::fs::create_dir_all(&hand).unwrap();
        std::fs::write(hand.join("templates.json"), b"hand").unwrap();
        std::fs::write(hand.join("notes.txt"), b"old").unwrap();
        let bytes = build_pack(name, "2", &[("templates.json".to_string(), b"{}".to_vec())]);
        let served = serve(
            vec![entry_json(name, "2", &bytes)],
            BTreeMap::from([(name.to_string(), (200, bytes))]),
        );
        let report = sync(&served, dir.path()).await;
        assert_eq!(
            outcome(&report, name),
            &PackSync::Failed(PACK_BAD_TEMPLATES.into()),
            "{name}"
        );
        assert!(is_fixed_refusal(PACK_BAD_TEMPLATES));
        assert_eq!(file_names(&hand), ["notes.txt", "templates.json"], "{name}");
        assert_eq!(std::fs::read(hand.join("templates.json")).unwrap(), b"hand");
        assert_eq!(kept_folders(dir.path()), Vec::<PathBuf>::new(), "{name}");
        assert_eq!(leftovers(dir.path()), Vec::<String>::new(), "{name}");
        assert!(!read_versions(dir.path()).contains_key(name), "{name}");
    }
}

/// The kept folder is named `<pack folder>-<UTC stamp>-<pid>`.
fn assert_kept_name(kept: &Path) {
    let name = kept.file_name().unwrap().to_string_lossy().into_owned();
    let rest = name.strip_prefix("rank-v1-").expect("pack label");
    let (stamp, pid) = rest.split_once('-').expect("stamp and pid");
    assert_eq!(pid, std::process::id().to_string(), "{name}");
    assert_eq!(stamp.len(), 16, "{name}");
    assert!(
        stamp.chars().enumerate().all(|(at, c)| match at {
            8 => c == 'T',
            15 => c == 'Z',
            _ => c.is_ascii_digit(),
        }),
        "{name}"
    );
}

/// Without the exchange (no staging folder, so it fails), the keep moves
/// the old folder under `templates-kept/` first and a swap that then fails
/// puts it back. With the exchange the new pack goes in first and the old
/// tree is moved out after.
#[test]
fn a_failed_swap_puts_the_kept_hand_folder_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hand = plant_hand_rank(dir.path());
    let staging = dir.path().join("templates").join(".rank-v1-next-gone");
    assert!(matches!(
        layout::swap_keeping(dir.path(), RANK_PACK_NAME, &staging, &hand),
        Err(layout::KeepSwap::NotSwapped(_))
    ));
    assert_eq!(kept_folders(dir.path()), Vec::<PathBuf>::new());
    assert_eq!(
        file_names(&hand),
        ["templates.json", "templates.json.bak-roles-20261114"]
    );
    assert_eq!(
        std::fs::read(hand.join("templates.json")).unwrap(),
        b"{\"hand\":true}"
    );

    let staging = dir.path().join("templates").join(".rank-v1-next-1");
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("templates.json"), rank_json()).unwrap();
    let kept = layout::swap_keeping(dir.path(), RANK_PACK_NAME, &staging, &hand).expect("swap");
    assert_eq!(kept_folders(dir.path()), std::slice::from_ref(&kept));
    assert_kept_name(&kept);
    assert!(!staging.exists());
    assert_eq!(file_names(&hand), ["templates.json"]);
    assert_eq!(
        file_names(&kept),
        ["templates.json", "templates.json.bak-roles-20261114"]
    );
}

/// A change made to the live folder, the files kept and the kept
/// `templates.json`.
type Case = (fn(&Path), &'static [&'static str], Vec<u8>);

/// After a first official install, the next version keeps the folder when
/// its `templates.json` was edited or a file was added, and replaces it
/// keeping nothing when it is as installed.
#[tokio::test]
async fn a_later_hand_edit_is_kept_and_an_unchanged_folder_is_replaced() {
    let edited =
        |live: &Path| std::fs::write(live.join("templates.json"), b"{\"edit\":1}").unwrap();
    let added = |live: &Path| std::fs::write(live.join("notes.txt"), b"mine").unwrap();
    let unchanged = |_: &Path| {};
    let edit = b"{\"edit\":1}".to_vec();
    let cases: [Case; 3] = [
        (edited, &["templates.json"], edit),
        (added, &["notes.txt", "templates.json"], rank_json()),
        (unchanged, &[], Vec::new()),
    ];
    for (change, kept_files, kept_templates) in cases {
        let dir = tempfile::tempdir().expect("tempdir");
        let live = dir.path().join(RANK_PACK_DIR);
        let report = sync(&serve_only(&[RANK_PACK_NAME], "1"), dir.path()).await;
        assert_eq!(outcome(&report, RANK_PACK_NAME), &PackSync::Installed);
        let hashes = std::fs::read(layout::file_hashes_path(dir.path())).expect("hashes");
        let hashes: BTreeMap<String, String> = serde_json::from_slice(&hashes).unwrap();
        assert_eq!(hashes[RANK_PACK_NAME], sha256_hex(&rank_json()));
        change(&live);
        let report = sync(&serve_only(&[RANK_PACK_NAME], "2"), dir.path()).await;
        assert_eq!(outcome(&report, RANK_PACK_NAME), &PackSync::Installed);
        assert_eq!(file_names(&live), ["templates.json"]);
        assert_eq!(
            std::fs::read(live.join("templates.json")).unwrap(),
            rank_json()
        );
        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
        let kept = kept_folders(dir.path());
        if kept_files.is_empty() {
            assert_eq!(kept, Vec::<PathBuf>::new());
            continue;
        }
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(file_names(&kept[0]), kept_files);
        assert_eq!(
            std::fs::read(kept[0].join("templates.json")).unwrap(),
            kept_templates
        );
    }
}

/// When `templates-kept/` cannot be made (a file has that name), the pack
/// is refused for a fixed reason, so the keeper does not fetch it again,
/// and the folder placed by hand stays as it was.
#[tokio::test]
async fn a_keep_that_cannot_happen_refuses_the_pack_and_leaves_the_folder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hand = plant_hand_rank(dir.path());
    std::fs::write(dir.path().join(layout::KEPT_DIR), b"in the way").unwrap();
    let report = sync(&serve_only(&[RANK_PACK_NAME], "1"), dir.path()).await;
    assert_eq!(
        outcome(&report, RANK_PACK_NAME),
        &PackSync::Failed(PACK_NOT_KEPT.into())
    );
    assert!(is_fixed_refusal(PACK_NOT_KEPT));
    assert_eq!(
        file_names(&hand),
        ["templates.json", "templates.json.bak-roles-20261114"]
    );
    assert_eq!(
        std::fs::read(hand.join("templates.json")).unwrap(),
        b"{\"hand\":true}"
    );
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
    assert!(!read_versions(dir.path()).contains_key(RANK_PACK_NAME));
}

/// Only a templates.json pack keeps: a hero folder placed by hand is
/// replaced as before.
#[test]
fn only_templates_json_packs_keep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let heroes = pack_spec(HEROES_PACK_NAME).unwrap();
    std::fs::create_dir_all(heroes.dir_in(dir.path())).unwrap();
    assert!(!layout::needs_keep(
        dir.path(),
        heroes,
        &heroes.dir_in(dir.path())
    ));
    let rank = pack_spec(RANK_PACK_NAME).unwrap();
    let hand = plant_hand_rank(dir.path());
    assert!(layout::needs_keep(dir.path(), rank, &hand));
    let missing = dir.path().join("templates/queue-title-v1");
    let queue_title = pack_spec(QUEUE_TITLE_PACK_NAME).unwrap();
    assert!(!layout::needs_keep(dir.path(), queue_title, &missing));
}

/// A templates.json pack is in place only when `templates.json` is a
/// regular file in its folder.
#[test]
fn a_templates_folder_is_in_place_only_with_its_templates_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec = pack_spec(RANK_PACK_NAME).unwrap();
    let live = dir.path().join(RANK_PACK_DIR);
    assert!(!in_place(dir.path(), spec));
    std::fs::create_dir_all(&live).unwrap();
    assert!(!in_place(dir.path(), spec), "an empty folder");
    std::fs::create_dir(live.join("templates.json")).unwrap();
    assert!(!in_place(dir.path(), spec), "a folder at that name");
    std::fs::remove_dir(live.join("templates.json")).unwrap();
    std::fs::write(live.join("real.json"), rank_json()).unwrap();
    std::os::unix::fs::symlink(live.join("real.json"), live.join("templates.json")).unwrap();
    assert!(!in_place(dir.path(), spec), "a link at that name");
    std::fs::remove_file(live.join("templates.json")).unwrap();
    std::fs::write(live.join("templates.json"), rank_json()).unwrap();
    assert!(in_place(dir.path(), spec));
}

#[tokio::test]
async fn a_list_without_the_packs_keeps_a_folder_placed_by_hand() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (sub, data) in [
        (RANK_PACK_DIR, rank_json()),
        (QUEUE_TITLE_PACK_DIR, queue_title_json()),
    ] {
        std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        std::fs::write(dir.path().join(sub).join("templates.json"), data).unwrap();
    }
    let (mut list, mut files, _) = all_packs();
    list.retain(|entry| entry["name"] != RANK_PACK_NAME && entry["name"] != QUEUE_TITLE_PACK_NAME);
    files.remove(RANK_PACK_NAME);
    files.remove(QUEUE_TITLE_PACK_NAME);
    let served = serve(list, files);
    let report = sync(&served, dir.path()).await;
    assert_eq!(outcome(&report, RANK_PACK_NAME), &PackSync::Missing);
    assert_eq!(outcome(&report, QUEUE_TITLE_PACK_NAME), &PackSync::Missing);
    assert_eq!(outcome(&report, "bans-picks-v1.tar"), &PackSync::Installed);
    assert_eq!(report.extras_ready(), (7, 7));
    assert!(both_load(dir.path()), "the hand-placed packs still load");
    let versions = stored_versions(dir.path());
    assert!(!versions.contains_key(RANK_PACK_NAME));
    assert!(!versions.contains_key(QUEUE_TITLE_PACK_NAME));
}

#[tokio::test]
async fn a_pack_with_another_file_is_refused_and_the_old_folder_stays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let live = dir.path().join(QUEUE_TITLE_PACK_DIR);
    std::fs::create_dir_all(&live).unwrap();
    std::fs::write(live.join("templates.json"), queue_title_json()).unwrap();
    let mut files = pack_files(QUEUE_TITLE_PACK_NAME);
    files.push(("notes.txt".to_string(), b"x".to_vec()));
    let bytes = build_pack(QUEUE_TITLE_PACK_NAME, "2", &files);
    let served = serve(
        vec![entry_json(QUEUE_TITLE_PACK_NAME, "2", &bytes)],
        BTreeMap::from([(QUEUE_TITLE_PACK_NAME.to_string(), (200, bytes))]),
    );
    let report = sync(&served, dir.path()).await;
    assert_eq!(
        outcome(&report, QUEUE_TITLE_PACK_NAME),
        &PackSync::Failed(PACK_UNSAFE.into())
    );
    assert_eq!(
        std::fs::read(live.join("templates.json")).unwrap(),
        queue_title_json()
    );
    assert!(!live.join("notes.txt").exists());
    assert!(queue_title::Reader::load(dir.path()).is_ok());
}

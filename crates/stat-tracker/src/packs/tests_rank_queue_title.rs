//! The rank-v1 and queue-title-v1 packs: a real unpack and sync of tars
//! built here from tiny synthetic `templates.json` files, the readers
//! loading what was installed, a folder placed by hand before the packs
//! were listed, and the cases where no pack arrives.

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

fn pack_files(name: &str) -> Vec<(String, Vec<u8>)> {
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

#[tokio::test]
async fn a_folder_placed_by_hand_is_replaced_whole_by_the_first_sync() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hand = dir.path().join(RANK_PACK_DIR);
    std::fs::create_dir_all(&hand).unwrap();
    std::fs::write(hand.join("templates.json"), b"{\"hand\":true}").unwrap();
    std::fs::write(hand.join("templates.json.bak-roles-20261114"), b"old").unwrap();
    // No version is recorded for a folder placed by hand, so it is fetched.
    let served = serve_only(&[RANK_PACK_NAME], "full-20261114");
    let report = sync(&served, dir.path()).await;
    assert_eq!(outcome(&report, RANK_PACK_NAME), &PackSync::Installed);
    let mut left: Vec<String> = std::fs::read_dir(&hand)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        ["templates.json"],
        "the extra file goes with the old folder"
    );
    assert_eq!(
        std::fs::read(hand.join("templates.json")).unwrap(),
        rank_json()
    );
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
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

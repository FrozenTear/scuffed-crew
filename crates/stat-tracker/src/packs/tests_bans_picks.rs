//! The bans-picks-v1 pack: a real unpack and sync of a tar built here, the
//! reader loading what was installed, and the cases where no pack arrives.

use super::layout::{BANS_PICKS_PACK_DIR, BANS_PICKS_PACK_NAME};
use super::*;
use crate::shadow::bans_picks::{
    NO_PREFERENCE_KEY, Reader, TEMPLATE_DIR, TEMPLATE_H, TEMPLATE_W, TemplateError, hero_keys,
};

fn template_png(seed: usize) -> Vec<u8> {
    let img = image::RgbImage::from_fn(TEMPLATE_W, TEMPLATE_H, |x, y| {
        let v = (x as usize * 7 + y as usize * 13 + seed * 31) % 256;
        image::Rgb([v as u8, (v / 2) as u8, seed as u8])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

/// One template per hero and the no-preference icon, 100x96 each.
fn full_pack_files() -> Vec<(String, Vec<u8>)> {
    let mut keys = hero_keys();
    keys.push(NO_PREFERENCE_KEY.to_string());
    keys.iter()
        .enumerate()
        .map(|(seed, key)| (format!("{key}.png"), template_png(seed)))
        .collect()
}

fn spec() -> &'static PackSpec {
    pack_spec(BANS_PICKS_PACK_NAME).expect("the bans-picks pack is listed")
}

#[test]
fn the_pack_lands_in_the_folder_the_reader_loads_from() {
    assert_eq!(BANS_PICKS_PACK_NAME, "bans-picks-v1.tar");
    assert_eq!(BANS_PICKS_PACK_DIR, TEMPLATE_DIR);
    assert!(!spec().required);
    let data = Path::new("/data");
    assert_eq!(
        spec().dir_in(data),
        data.join(TEMPLATE_DIR),
        "the pack folder and the reader folder are one"
    );
}

#[tokio::test]
async fn a_real_pack_unpacks_into_templates_and_the_reader_loads_it() {
    let files = full_pack_files();
    let bytes = build_pack(BANS_PICKS_PACK_NAME, "1", &files);
    let list = vec![entry_json(BANS_PICKS_PACK_NAME, "1", &bytes)];
    let served = serve(
        list,
        BTreeMap::from([(BANS_PICKS_PACK_NAME.to_string(), (200, bytes))]),
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let report = sync(&served, dir.path()).await;
    assert_eq!(outcome(&report, BANS_PICKS_PACK_NAME), &PackSync::Installed);
    let live = dir.path().join(TEMPLATE_DIR);
    for (path, data) in &files {
        assert_eq!(
            &std::fs::read(live.join(path)).unwrap_or_default(),
            data,
            "{path}"
        );
    }
    assert!(
        !live.join("manifest.json").exists(),
        "only names-v1 keeps its manifest"
    );
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
    assert_eq!(stored_versions(dir.path())[BANS_PICKS_PACK_NAME], "1");
    assert!(
        Reader::load(dir.path()).is_ok(),
        "the reader loads what the sync installed"
    );
}

#[tokio::test]
async fn a_pack_that_misses_a_hero_installs_but_the_reader_refuses_it() {
    // A hero added to the game and not yet in the published pack.
    let mut files = full_pack_files();
    let (gone, _) = files.remove(0);
    let bytes = build_pack(BANS_PICKS_PACK_NAME, "1", &files);
    let list = vec![entry_json(BANS_PICKS_PACK_NAME, "1", &bytes)];
    let served = serve(
        list,
        BTreeMap::from([(BANS_PICKS_PACK_NAME.to_string(), (200, bytes))]),
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let report = sync(&served, dir.path()).await;
    assert_eq!(outcome(&report, BANS_PICKS_PACK_NAME), &PackSync::Installed);
    match Reader::load(dir.path()) {
        Err(TemplateError::Missing(keys)) => {
            assert_eq!(keys, [gone.trim_end_matches(".png").to_string()]);
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(_) => panic!("a pack without a hero must not load"),
    }
}

#[tokio::test]
async fn a_list_without_the_pack_installs_the_rest_and_leaves_no_reader_folder() {
    let (mut list, mut files, _) = all_packs();
    list.retain(|entry| entry["name"] != BANS_PICKS_PACK_NAME);
    files.remove(BANS_PICKS_PACK_NAME);
    let served = serve(list, files);
    let dir = tempfile::tempdir().expect("tempdir");
    let report = sync(&served, dir.path()).await;
    assert_eq!(outcome(&report, BANS_PICKS_PACK_NAME), &PackSync::Missing);
    assert_eq!(outcome(&report, "bans-v1.tar"), &PackSync::Installed);
    assert!(!dir.path().join(TEMPLATE_DIR).exists());
    assert!(matches!(
        Reader::load(dir.path()),
        Err(TemplateError::Missing(_))
    ));
    assert_eq!(report.extras_ready(), (8, 8));
    assert!(!stored_versions(dir.path()).contains_key(BANS_PICKS_PACK_NAME));
}

#[tokio::test]
async fn without_a_site_token_no_pack_is_fetched_and_the_reader_has_nothing_to_load() {
    let (list, files, _) = all_packs();
    let served = serve(list, files);
    let dir = tempfile::tempdir().expect("tempdir");
    // An install that is not linked to a site account has no token. The
    // pack routes answer 401 to any other token.
    let report = sync_reader_packs(served.base.as_str(), "", dir.path(), PACK_MAX_BYTES).await;
    assert!(matches!(
        outcome(&report, BANS_PICKS_PACK_NAME),
        PackSync::Failed(_)
    ));
    assert_eq!(served.hits(), 0, "no pack was downloaded");
    assert!(!dir.path().join(TEMPLATE_DIR).exists());
    assert!(matches!(
        Reader::load(dir.path()),
        Err(TemplateError::Missing(_))
    ));
}

#[tokio::test]
async fn a_keeper_with_no_token_sends_nothing_and_says_to_sign_in() {
    let (list, files, _) = all_packs();
    let served = serve(list, files);
    let dir = tempfile::tempdir().expect("tempdir");
    let rig = rig_with(|| None, dir.path(), None, 1, no_nap);
    // The keeper waits and asks again; the first wait never ends here.
    let (finished, waits) = rig.run().await;
    assert!(!finished);
    assert_eq!(waits, [secs(60), secs(300)]);
    assert_eq!(requests(&served), Vec::<String>::new());
    assert!(!dir.path().join(TEMPLATE_DIR).exists());
}

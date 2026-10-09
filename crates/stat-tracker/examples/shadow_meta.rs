//! Eval probe: map banner and result word reads of the shadow reader.
//! Usage: shadow_meta <pack-root> <img>...  (pack-root holds templates/{maps,result})
//! Prints TSV: path, map key, map score, margin, suspect, result, score, margin, suspect.

use std::path::Path;

use stat_tracker::shadow::banner::MapTemplates;
use stat_tracker::shadow::result::ResultTemplates;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args.next().expect("pack root");
    let maps = MapTemplates::load_dir(&MapTemplates::dir_in(Path::new(&root))).expect("map pack");
    let res =
        ResultTemplates::load_dir(&ResultTemplates::dir_in(Path::new(&root))).expect("result pack");
    for path in args {
        let Ok(img) = image::open(&path) else {
            println!("{path}\tERR");
            continue;
        };
        let rgb = img.to_rgb8();
        let m = maps.read(&rgb);
        let r = res.read(&rgb);
        println!(
            "{path}\t{}\t{:.3}\t{:.3}\t{}\t{}\t{:.3}\t{:.3}\t{}",
            m.map.map_or("-", |i| i.key),
            m.score,
            m.margin,
            m.suspect,
            r.outcome.map_or("-", |o| o.as_str()),
            r.score,
            r.margin,
            r.suspect
        );
    }
}

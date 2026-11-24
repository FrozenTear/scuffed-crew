//! Every `toml` block in the tracker README (`README.md` of this package)
//! and the desktop app README (`../stat-tracker-ui/README.md`) is a
//! `config.toml` example, and each must load as written. A copied example
//! that does not parse leaves the tracker unable to start.
//!
//! The README sits next to this crate's `Cargo.toml`, like the release pins
//! in `tests_release_pins.rs`.

use crate::config::Config;

const READMES: [(&str, &str); 2] = [
    (
        "crates/stat-tracker/README.md",
        include_str!("../README.md"),
    ),
    (
        "crates/stat-tracker-ui/README.md",
        include_str!("../../stat-tracker-ui/README.md"),
    ),
];

/// The opening fence of a fenced code block: the fence itself (three or
/// more backticks or tildes) and the block's language, lowercased.
fn open_fence(line: &str) -> Option<(String, String)> {
    let line = line.trim_start();
    let mark = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = line.chars().take_while(|c| *c == mark).count();
    if len < 3 {
        return None;
    }
    let fence = line[..len].to_string();
    let lang = line[len..]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    Some((fence, lang))
}

/// The bodies of the fenced `toml` blocks in `markdown`, in order. Accepts
/// any case and a space before the language (```` ``` TOML ````), and
/// tilde fences (`~~~toml`).
fn toml_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    // The open block: its fence, whether it is toml, and its body.
    let mut open: Option<(String, bool, String)> = None;
    for line in markdown.lines() {
        match open.as_mut() {
            None => {
                if let Some((fence, lang)) = open_fence(line) {
                    open = Some((fence, lang == "toml", String::new()));
                }
            }
            Some((fence, is_toml, body)) => {
                let t = line.trim();
                let closes =
                    t.starts_with(fence.as_str()) && t.chars().all(|c| fence.starts_with(c));
                if closes {
                    if *is_toml {
                        blocks.push(std::mem::take(body));
                    }
                    open = None;
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    assert!(open.is_none(), "unclosed code block in README");
    blocks
}

#[test]
fn every_readme_config_example_loads() {
    let mut total = 0;
    for (path, readme) in READMES {
        for block in toml_blocks(readme) {
            total += 1;
            let config = Config::parse_file_contents(&block)
                .unwrap_or_else(|e| panic!("{path}: config example does not load: {e}\n{block}"));
            assert!(
                config.data_dir.is_absolute(),
                "{path}: example data_dir must be absolute:\n{block}"
            );
        }
    }
    assert!(total > 0, "the READMEs have no toml config example");
}

#[test]
fn readme_low_ram_example_sets_one_ocr_thread() {
    let readme = READMES[0].1;
    let (_, after) = readme
        .split_once("Example low-RAM")
        .expect("README low-RAM example");
    let block = toml_blocks(after)
        .into_iter()
        .next()
        .expect("a toml block after the low-RAM heading");
    let config = Config::parse_file_contents(&block).expect("low-RAM example loads");
    assert_eq!(config.ocr_threads, Some(1), "{block}");
}

#[test]
fn toml_blocks_reads_every_fence_form() {
    let md = "```sh\nx = 1\n```\n\
              ```toml\na = 1\n```\n\
              ```TOML\nb = 2\n```\n\
              ``` toml\nc = 3\n```\n\
              ~~~toml\nd = 4\n~~~\n\
              ````toml\ne = \"```\"\n````\n";
    assert_eq!(
        toml_blocks(md),
        vec![
            "a = 1\n".to_string(),
            "b = 2\n".to_string(),
            "c = 3\n".to_string(),
            "d = 4\n".to_string(),
            "e = \"```\"\n".to_string(),
        ]
    );
}

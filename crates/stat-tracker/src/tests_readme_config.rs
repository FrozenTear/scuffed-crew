//! Every `toml` block in this package's `README.md` is a `config.toml`
//! example, and each must load as written. A copied example that does not
//! parse leaves the tracker unable to start.
//!
//! The README sits next to this crate's `Cargo.toml`, like the release pins
//! in `tests_release_pins.rs`.

use crate::config::Config;

/// The bodies of the fenced ```` ```toml ```` blocks in `markdown`, in order.
fn toml_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        let fence = line.trim_start();
        match current.as_mut() {
            None if fence.starts_with("```toml") => current = Some(String::new()),
            Some(body) if fence.starts_with("```") => {
                blocks.push(std::mem::take(body));
                current = None;
            }
            Some(body) => {
                body.push_str(line);
                body.push('\n');
            }
            None => {}
        }
    }
    assert!(current.is_none(), "unclosed ```toml block in README");
    blocks
}

#[test]
fn every_readme_config_example_loads() {
    let readme = include_str!("../README.md");
    let blocks = toml_blocks(readme);
    assert!(!blocks.is_empty(), "README has no ```toml config example");
    for block in &blocks {
        let config = Config::parse_file_contents(block)
            .unwrap_or_else(|e| panic!("README config example does not load: {e}\n{block}"));
        assert!(
            config.data_dir.is_absolute(),
            "README example data_dir must be absolute:\n{block}"
        );
    }
}

#[test]
fn readme_low_ram_example_sets_one_ocr_thread() {
    let readme = include_str!("../README.md");
    let (_, after) = readme
        .split_once("Example low-RAM")
        .expect("README low-RAM example");
    let block = toml_blocks(after)
        .into_iter()
        .next()
        .expect("a ```toml block after the low-RAM heading");
    let config = Config::parse_file_contents(&block).expect("low-RAM example loads");
    assert_eq!(config.ocr_threads, Some(1), "{block}");
}

#[test]
fn toml_blocks_skips_other_languages() {
    let md = "```sh\nx = 1\n```\ntext\n```toml\na = 1\n```\n";
    assert_eq!(toml_blocks(md), vec!["a = 1\n".to_string()]);
}

//! Tests for `memtune.rs`.

use super::memtune::*;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: std::collections::HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| map.get(k).cloned()
}

#[test]
fn sets_all_three_limits_by_default() {
    let a = plan(env(&[]));
    assert!(!a.disabled);
    assert_eq!(a.set, vec!["arena_max", "mmap_threshold", "trim_threshold"]);
    assert!(a.user.is_empty());
    assert_eq!(
        (ARENA_MAX, MMAP_THRESHOLD, TRIM_THRESHOLD),
        (2, 1 << 20, 4 << 20)
    );
}

#[test]
fn a_limit_the_user_set_wins() {
    let a = plan(env(&[
        ("MALLOC_ARENA_MAX", "4"),
        (
            "GLIBC_TUNABLES",
            "glibc.malloc.trim_threshold=1:glibc.foo=2",
        ),
    ]));
    assert_eq!(a.set, vec!["mmap_threshold"]);
    assert_eq!(a.user, vec!["arena_max", "trim_threshold"]);
    let blank = plan(env(&[("MALLOC_ARENA_MAX", " ")]));
    assert_eq!(blank.set.len(), 3, "an empty variable sets nothing");
}

#[test]
fn memtune_zero_turns_it_off() {
    let a = plan(env(&[("STAT_TRACKER_MEMTUNE", "0")]));
    assert!(a.disabled);
    assert!(a.set.is_empty());
    assert_eq!(plan(env(&[("STAT_TRACKER_MEMTUNE", "1")])).set.len(), 3);
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn heap_and_rss_read() {
    assert!(live_heap_bytes().is_some_and(|b| b > 0));
    assert!(resident_bytes().is_some_and(|b| b > 0));
    trim();
}

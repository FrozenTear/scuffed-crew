//! The page background cascade: the boot shell styles in `index.html` and the
//! app's own `html, body` rule (`theme/tokens.rs`), run through a small CSS
//! cascade so the tests check which rule paints, not how the rules are spelt.
//!
//! Vision 694 r3 blocking 1: the saved-theme boot rules out-ranked the app
//! rule, and `ThemeProvider` keeps `data-theme` set for the whole visit, so a
//! page background colour or image set in Admin Settings never showed.
//!
//! The cascade covers what these two style sheets use: type, id, class and
//! attribute selectors, `:where()`, the descendant combinator, the
//! `prefers-color-scheme` media query, and the `background` shorthand.

use std::collections::HashMap;

/// One element on the path from the root: tag and attributes.
#[derive(Clone, Debug, Default)]
struct Node {
    tag: &'static str,
    attrs: HashMap<&'static str, &'static str>,
}

/// A selector rule with the media condition it sits under, in source order.
#[derive(Debug)]
struct Rule {
    selectors: Vec<String>,
    decls: Vec<(String, String)>,
    /// `Some(true)` under `prefers-color-scheme: light`, `Some(false)` under
    /// dark, `None` unconditional.
    light_only: Option<bool>,
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = match rest[start + 2..].find("*/") {
            Some(end) => &rest[start + 2 + end + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The text up to the brace that closes the block `body` starts inside.
fn block(body: &str) -> (&str, &str) {
    let mut depth = 1usize;
    for (i, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (&body[..i], &body[i + 1..]);
                }
            }
            _ => {}
        }
    }
    (body, "")
}

fn parse_rules(css: &str, light_only: Option<bool>, out: &mut Vec<Rule>) {
    let mut rest = css;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return;
        }
        let Some(open) = rest.find('{') else {
            return;
        };
        let prelude = rest[..open].trim();
        // A statement at-rule (`@import ...;`) before the next block.
        if prelude.starts_with('@')
            && let Some(semi) = prelude.find(';')
        {
            rest = &rest[semi + 1..];
            continue;
        }
        let (inner, after) = block(&rest[open + 1..]);
        rest = after;
        if let Some(at) = prelude.strip_prefix('@') {
            let scheme = if at.contains("prefers-color-scheme: light") {
                Some(true)
            } else if at.contains("prefers-color-scheme: dark") {
                Some(false)
            } else {
                // Other at-rules (font faces, motion, widths) never touch the
                // page background in these sheets; skip them whole.
                continue;
            };
            parse_rules(inner, scheme, out);
            continue;
        }
        let decls = inner
            .split(';')
            .filter_map(|d| {
                let (k, v) = d.split_once(':')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        out.push(Rule {
            selectors: split_top(prelude, ',')
                .into_iter()
                .map(|s| s.trim().to_string())
                .collect(),
            decls,
            light_only,
        });
    }
}

/// Split on `sep` outside brackets and parentheses.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            c if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// The compounds of a selector, left to right (descendant combinators only).
fn compounds(sel: &str) -> Vec<&str> {
    split_top(sel, ' ')
        .into_iter()
        .filter(|c| !c.is_empty())
        .collect()
}

/// Specificity `(ids, classes and attributes, types)` of one compound, and
/// whether it matches `node`.
fn compound(c: &str, node: &Node) -> ((u32, u32, u32), bool) {
    let mut spec = (0, 0, 0);
    let mut ok = true;
    let mut rest = c;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix(":where(") {
            let (inner, after) = paren(r);
            // Zero specificity; matches when any listed compound chain
            // matches this element.
            ok &= split_top(inner, ',').into_iter().any(|alt| {
                let parts = compounds(alt.trim());
                parts.len() == 1 && compound(parts[0], node).1
            });
            rest = after;
        } else if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').expect("closed attribute selector");
            let (name, want) = match r[..end].split_once('=') {
                Some((n, v)) => (n, Some(v.trim_matches('"'))),
                None => (&r[..end], None),
            };
            ok &= match (node.attrs.get(name), want) {
                (Some(v), Some(w)) => *v == w,
                (Some(_), None) => true,
                (None, _) => false,
            };
            spec.1 += 1;
            rest = &r[end + 1..];
        } else if let Some(r) = rest.strip_prefix('#') {
            let end = r.find(['.', '[', ':', '#']).unwrap_or(r.len());
            ok &= node.attrs.get("id") == Some(&&r[..end]);
            spec.0 += 1;
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('.') {
            let end = r.find(['.', '[', ':', '#']).unwrap_or(r.len());
            ok &= node
                .attrs
                .get("class")
                .is_some_and(|cls| cls.split(' ').any(|k| k == &r[..end]));
            spec.1 += 1;
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix(':') {
            // Other pseudo-classes and pseudo-elements: `:root` is `html`;
            // the rest (hover, focus, ::before) never paint the page itself.
            let r = r.trim_start_matches(':');
            let end = r.find(['.', '[', ':', '#', '(']).unwrap_or(r.len());
            ok &= &r[..end] == "root" && node.tag == "html";
            spec.1 += 1;
            rest = match r[end..].strip_prefix('(') {
                Some(args) => paren(args).1,
                None => &r[end..],
            };
        } else {
            let end = rest.find(['.', '[', ':', '#']).unwrap_or(rest.len());
            let tag = &rest[..end];
            if tag != "*" {
                ok &= node.tag == tag;
                spec.2 += 1;
            }
            rest = &rest[end..];
        }
    }
    (spec, ok)
}

fn paren(s: &str) -> (&str, &str) {
    let mut depth = 1;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (&s[..i], &s[i + 1..]);
                }
            }
            _ => {}
        }
    }
    panic!("unclosed paren in {s}");
}

/// Specificity of `sel` and whether it matches the last node of `path`.
fn matches(sel: &str, path: &[Node]) -> Option<(u32, u32, u32)> {
    let parts = compounds(sel);
    let (last, ancestors) = parts.split_last()?;
    let (mut spec, ok) = compound(last, path.last()?);
    if !ok {
        return None;
    }
    // Descendant: match each earlier compound against some ancestor, right
    // to left, greedily (enough for these sheets).
    let mut up = path.len() - 1;
    for part in ancestors.iter().rev() {
        let mut found = false;
        while up > 0 {
            up -= 1;
            let (s, ok) = compound(part, &path[up]);
            if ok {
                spec = (spec.0 + s.0, spec.1 + s.1, spec.2 + s.2);
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }
    Some(spec)
}

/// The winning `background-color` and `background-image` for the last node
/// of `path`, with the OS in light or dark.
fn background(rules: &[Rule], path: &[Node], os_light: bool) -> (String, String) {
    // (specificity, source order, value)
    let mut color: Option<((u32, u32, u32), usize, String)> = None;
    let mut image: Option<((u32, u32, u32), usize, String)> = None;
    let mut order = 0usize;
    for rule in rules {
        if rule.light_only.is_some_and(|l| l != os_light) {
            continue;
        }
        let Some(spec) = rule.selectors.iter().filter_map(|s| matches(s, path)).max() else {
            continue;
        };
        for (prop, value) in &rule.decls {
            order += 1;
            let set = |slot: &mut Option<_>, v: String| {
                let beats =
                    slot.as_ref()
                        .is_none_or(|(s, o, _): &((u32, u32, u32), usize, String)| {
                            (spec, order) > (*s, *o)
                        });
                if beats {
                    *slot = Some((spec, order, v));
                }
            };
            match prop.as_str() {
                "background-color" => set(&mut color, value.clone()),
                "background-image" => set(&mut image, value.clone()),
                // The shorthand resets every longhand it does not name.
                "background" => {
                    let (img, col): (Vec<&str>, Vec<&str>) = value
                        .split_whitespace()
                        .partition(|w| w.starts_with("url(") || w.contains("gradient("));
                    set(&mut image, img.first().map_or("none", |v| v).to_string());
                    set(&mut color, col.join(" "));
                }
                _ => {}
            }
        }
    }
    let pick = |slot: Option<(_, _, String)>, initial: &str| {
        slot.map_or_else(|| initial.to_string(), |(_, _, v)| v)
    };
    (pick(color, "transparent"), pick(image, "none"))
}

/// The `<style>` in `index.html`, then the app's theme css, in document order
/// (the app's `style` element renders inside `#main`, after `<head>`).
fn rules(with_app: bool) -> Vec<Rule> {
    let html = include_str!("../index.html");
    let start = html.find("<style>").expect("shell style") + "<style>".len();
    let end = html[start..].find("</style>").expect("closed style") + start;
    let mut out = Vec::new();
    parse_rules(&strip_css_comments(&html[start..end]), None, &mut out);
    if with_app {
        let app = crate::theme::tokens::theme_css(&crate::theme::brand::current());
        parse_rules(&strip_css_comments(&app), None, &mut out);
    }
    out
}

fn page(theme: Option<&'static str>) -> [Vec<Node>; 2] {
    let mut root = Node {
        tag: "html",
        ..Node::default()
    };
    if let Some(theme) = theme {
        root.attrs.insert("data-theme", theme);
    }
    let body = Node {
        tag: "body",
        ..Node::default()
    };
    [vec![root.clone()], vec![root, body]]
}

const APP_COLOR: &str = "var(--page-bg-color, var(--bg))";
const APP_IMAGE: &str = "var(--page-bg-image, none)";

/// Vision 694 r3 blocking 1: once the app runs, `data-theme` is always set,
/// and the app's page rule must still paint `html` and `body`, so a page
/// background colour and image from Admin Settings show in both themes and
/// whatever the OS prefers.
#[test]
fn the_app_page_background_wins_over_the_boot_rules() {
    let rules = rules(true);
    for theme in ["dark", "light"] {
        for os_light in [false, true] {
            for (who, path) in ["html", "body"].iter().zip(page(Some(theme))) {
                let (color, image) = background(&rules, &path, os_light);
                assert_eq!(
                    (color.as_str(), image.as_str()),
                    (APP_COLOR, APP_IMAGE),
                    "{who}, data-theme={theme}, OS light={os_light}"
                );
            }
        }
    }
}

/// Before the app loads only the shell styles apply. A saved theme
/// (`boot-theme.js` sets `data-theme`) beats the OS; no saved theme follows
/// the OS. The colours are the Look A `--bg` of that theme.
#[test]
fn the_boot_splash_paints_the_saved_theme_then_the_os() {
    let rules = rules(false);
    let app_css = crate::theme::tokens::theme_css(&crate::theme::brand::current());
    let bg = |theme: &str| crate::theme::tokens::resolved_tokens(&app_css, theme)["--bg"].clone();
    let cases = [
        (Some("dark"), false, "dark"),
        (Some("dark"), true, "dark"),
        (Some("light"), false, "light"),
        (Some("light"), true, "light"),
        (None, false, "dark"),
        (None, true, "light"),
    ];
    for (saved, os_light, want) in cases {
        for (who, path) in ["html", "body"].iter().zip(page(saved)) {
            let (color, image) = background(&rules, &path, os_light);
            assert_eq!(
                (color.as_str(), image.as_str()),
                (bg(want).as_str(), "none"),
                "{who}, saved={saved:?}, OS light={os_light}"
            );
        }
    }
}

/// The cascade model itself: specificity beats order, `:where()` weighs
/// nothing, and the shorthand resets the image.
#[test]
fn the_test_cascade_follows_css_rules() {
    let mut rules = Vec::new();
    parse_rules(
        "html[data-theme=\"dark\"] body { background: #111; }
         html, body { background-color: red; background-image: url(x.png); }",
        None,
        &mut rules,
    );
    let [_, body] = page(Some("dark"));
    assert_eq!(
        background(&rules, &body, false),
        ("#111".into(), "none".into())
    );

    let mut rules = Vec::new();
    parse_rules(
        ":where(html[data-theme=\"dark\"]) body { background: #111; }
         @media (prefers-color-scheme: light) { body { background: #eee; } }
         html, body { background-color: red; background-image: url(x.png); }",
        None,
        &mut rules,
    );
    assert_eq!(
        background(&rules, &body, true),
        ("red".into(), "url(x.png)".into())
    );
    assert_eq!(
        matches(":where(html[data-theme=\"dark\"]) body", &body),
        Some((0, 0, 1))
    );
    assert_eq!(matches("#sc-boot .label", &body), None);
}

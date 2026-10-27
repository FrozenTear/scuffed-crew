//! Right panel: one hero's details.
//!
//! The header (portrait, name, role) always shows. The HP bar and the
//! Abilities list only show when the hero list sends them; a short muted
//! note shows only when the list sent neither.

use dioxus::prelude::*;

use super::model::{Ability, Hero};
use super::portrait::HeroPortrait;

pub(super) fn render_hero_detail(hero: &Hero) -> Element {
    let group = hero.group();
    let role_color = group.color();
    let role_tint = group.tint();
    let role_label = group.label();

    rsx! {
        div { class: "hero-detail-header",
            HeroPortrait {
                url: hero.portrait().map(str::to_string),
                name: hero.name.clone(),
                initial: hero.initial(),
                group,
                class: "hero-detail-portrait",
                initial_class: "hero-detail-portrait-initial",
            }
            div { class: "hero-detail-info",
                h2 { class: "hero-detail-name", "{hero.name}" }
                span {
                    class: "hero-detail-role-badge",
                    style: "background: {role_tint}; color: {role_color};",
                    "{role_label}"
                }
            }
        }

        if hero.has_hp_detail() {
            {render_hp(hero)}
        }

        if hero.has_abilities() {
            h3 { class: "hero-abilities-title", "Abilities" }
            div { class: "hero-abilities-list",
                for ability in hero.abilities.iter() {
                    {render_ability_card(ability)}
                }
            }
        }

        if !hero.has_details() {
            p { class: "hero-detail-pending", "More details soon." }
        }
    }
}

/// Percent of `total` for one HP segment.
fn pct(part: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(part) / f64::from(total) * 100.0
    }
}

fn render_hp(hero: &Hero) -> Element {
    let total = hero.total_hp();
    let segments: Vec<(&str, &str, u32, f64)> = [
        ("Health", "var(--text-2)", hero.health),
        ("Armor", "var(--warn)", hero.armor),
        ("Shields", "var(--chart-5)", hero.shields),
    ]
    .into_iter()
    .filter_map(|(label, color, value)| {
        value
            .filter(|v| *v > 0)
            .map(|v| (label, color, v, pct(v, total)))
    })
    .collect();

    rsx! {
        div { class: "hero-hp-section",
            div { class: "hero-hp-label", "Hit Points {total}" }
            div { class: "hero-hp-bar",
                for (label, color, value, width) in segments.iter() {
                    div {
                        key: "{label}",
                        class: "hero-hp-segment",
                        style: "width:{width:.1}%; background:{color};",
                        "{value}"
                    }
                }
            }
            div { class: "hero-hp-legend",
                for (label, color, value, _) in segments.iter() {
                    span { key: "{label}",
                        span { class: "hero-hp-dot", style: "background:{color};" }
                        "{label} {value}"
                    }
                }
            }
        }
    }
}

fn cooldown_text(cooldown: Option<f32>) -> Option<String> {
    cooldown.filter(|cd| *cd > 0.0).map(|cd| {
        if cd == cd.floor() {
            format!("{cd:.0}s")
        } else {
            format!("{cd:.1}s")
        }
    })
}

fn render_ability_card(ability: &Ability) -> Element {
    let cooldown = cooldown_text(ability.cooldown);

    rsx! {
        div { class: "ability-card", key: "{ability.key}-{ability.name}",
            if !ability.key.is_empty() {
                div { class: "ability-key-badge", "{ability.key}" }
            }
            div { class: "ability-body",
                div { class: "ability-header",
                    span { class: "ability-name", "{ability.name}" }
                    if let Some(cd) = &cooldown {
                        span { class: "ability-cooldown", "{cd}" }
                    }
                }
                if !ability.description.is_empty() {
                    p { class: "ability-desc", "{ability.description}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    fn decode(body: &str) -> Hero {
        serde_json::from_str(body).unwrap()
    }

    #[component]
    fn ListOnly() -> Element {
        render_hero_detail(&decode(
            r#"{"id":"doctrine","name":"Doctrine","role":"support","portrait_url":""}"#,
        ))
    }

    #[component]
    fn WithDetails() -> Element {
        render_hero_detail(&decode(
            r#"{"id":"ana","name":"Ana","role":"support","portrait_url":"/img/ana.png",
            "health":250,"abilities":[{"name":"Sleep Dart","key":"Shift","cooldown":14}]}"#,
        ))
    }

    #[test]
    fn list_only_hero_shows_header_and_fallback_only() {
        let html = render(ListOnly);
        assert!(html.contains("Doctrine"), "{html}");
        assert!(html.contains("Support"), "{html}");
        assert!(
            !html.contains("<img"),
            "blank portrait draws the initial: {html}"
        );
        assert!(html.contains("hero-detail-portrait-initial"), "{html}");
        assert!(!html.contains("Abilities"), "{html}");
        assert!(!html.contains("Hit Points"), "{html}");
        assert!(html.contains("More details soon."), "{html}");
    }

    #[test]
    fn present_details_render_their_sections() {
        let html = render(WithDetails);
        assert!(html.contains("<img"), "{html}");
        assert!(html.contains("Hit Points 250"), "{html}");
        assert!(html.contains("Health 250"), "{html}");
        assert!(
            !html.contains("Armor"),
            "zero or missing segments are skipped: {html}"
        );
        assert!(html.contains("Sleep Dart"), "{html}");
        assert!(html.contains("14s"), "{html}");
        assert!(
            !html.contains("ability-desc"),
            "empty description is skipped: {html}"
        );
        assert!(!html.contains("More details soon."), "{html}");
    }

    #[component]
    fn HpOnly() -> Element {
        render_hero_detail(&decode(
            r#"{"id":"rein","name":"Reinhardt","role":"tank","portrait_url":"","armor":300}"#,
        ))
    }

    #[component]
    fn AbilitiesOnly() -> Element {
        render_hero_detail(&decode(
            r#"{"id":"mercy","name":"Mercy","role":"support","portrait_url":"",
            "abilities":[{"name":"Guardian Angel"}]}"#,
        ))
    }

    #[test]
    fn pending_note_needs_no_abilities_and_no_stats() {
        let hp_only = render(HpOnly);
        assert!(hp_only.contains("Hit Points 300"), "{hp_only}");
        assert!(!hp_only.contains("More details soon."), "{hp_only}");

        let abilities_only = render(AbilitiesOnly);
        assert!(
            abilities_only.contains("Guardian Angel"),
            "{abilities_only}"
        );
        assert!(
            !abilities_only.contains("More details soon."),
            "{abilities_only}"
        );
    }

    #[test]
    fn cooldowns_format_and_skip_zero() {
        assert_eq!(cooldown_text(Some(14.0)).as_deref(), Some("14s"));
        assert_eq!(cooldown_text(Some(1.5)).as_deref(), Some("1.5s"));
        assert_eq!(cooldown_text(Some(0.0)), None);
        assert_eq!(cooldown_text(None), None);
    }

    #[test]
    fn hp_percent_handles_zero_total() {
        assert_eq!(pct(5, 0), 0.0);
        assert_eq!(pct(50, 200), 25.0);
    }
}

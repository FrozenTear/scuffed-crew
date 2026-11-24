//! `GET /api/strategy/heroes`: the shared hero list for the strategy pages.
//!
//! Served from [`scuffed_types::HERO_NAMES`], the same table stats uploads
//! validate against, so the planner and the stats pages agree on ids, names,
//! and roles. Public read (no login), gated only by `strategies_enabled` in
//! the strategy router. The list is static per build, so the response is
//! cacheable.

use axum::{
    Json,
    http::header,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use scuffed_types::strategy::HeroRole;
use scuffed_types::{HERO_NAMES, HeroName, pack_file_stem};

/// Browsers and proxies may reuse the list for five minutes.
pub(crate) const HEROES_CACHE_CONTROL: &str = "public, max-age=300";

/// One hero in the strategy list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StrategyHero {
    /// Pack file stem, for example `wrecking-ball` or `soldier-76`.
    pub id: &'static str,
    /// Display name, for example `Wrecking Ball` or `Lúcio`.
    pub name: &'static str,
    /// Current role, lowercase: `tank`, `damage`, or `support`.
    pub role: HeroRole,
    /// Portrait URL. Game art is not in git, so this is empty and the page
    /// shows the hero's initial.
    pub portrait_url: &'static str,
}

impl From<&'static HeroName> for StrategyHero {
    fn from(hero: &'static HeroName) -> Self {
        Self {
            id: pack_file_stem(hero.file),
            name: hero.display,
            role: hero.role,
            portrait_url: "",
        }
    }
}

#[derive(Debug, Serialize)]
struct HeroList {
    data: Vec<StrategyHero>,
}

/// Every hero in [`HERO_NAMES`] order, which is stable across requests.
pub(crate) fn strategy_heroes() -> Vec<StrategyHero> {
    HERO_NAMES.iter().map(StrategyHero::from).collect()
}

/// GET /api/strategy/heroes: `{ "data": [{ id, name, role, portrait_url }] }`.
pub(crate) async fn list_heroes() -> Response {
    (
        [(header::CACHE_CONTROL, HEROES_CACHE_CONTROL)],
        Json(HeroList {
            data: strategy_heroes(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::{Router, routing::get};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    async fn fetch() -> (StatusCode, Option<String>, Value) {
        let app = Router::new().route("/api/strategy/heroes", get(list_heroes));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/strategy/heroes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let cache = resp
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_string());
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, cache, serde_json::from_slice(&bytes).unwrap())
    }

    fn data(body: &Value) -> &Vec<Value> {
        body["data"].as_array().expect("{ data: [...] } envelope")
    }

    #[tokio::test]
    async fn serves_every_hero_with_public_cache_header() {
        let (status, cache, body) = fetch().await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(cache.as_deref(), Some(HEROES_CACHE_CONTROL));
        let heroes = data(&body);
        assert_eq!(heroes.len(), HERO_NAMES.len());
        assert!(!heroes.is_empty());
        for hero in heroes {
            let keys: HashSet<&str> = hero
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                HashSet::from(["id", "name", "role", "portrait_url"]),
                "{hero}"
            );
            assert_eq!(hero["portrait_url"], "", "{hero}");
        }
    }

    #[tokio::test]
    async fn ids_names_and_roles_follow_the_shared_table_in_order() {
        let (_, _, body) = fetch().await;
        let heroes = data(&body);
        for (got, want) in heroes.iter().zip(HERO_NAMES) {
            assert_eq!(got["id"], pack_file_stem(want.file));
            assert_eq!(got["name"], want.display);
            let role = want.role.to_string().to_lowercase();
            assert_eq!(got["role"], role.as_str());
        }
        let ids: HashSet<&str> = heroes.iter().map(|h| h["id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), heroes.len(), "ids must be unique");
    }

    #[tokio::test]
    async fn roles_are_lowercase_and_known() {
        let (_, _, body) = fetch().await;
        for hero in data(&body) {
            let role = hero["role"].as_str().unwrap();
            assert!(["tank", "damage", "support"].contains(&role), "{hero}");
        }
    }

    #[tokio::test]
    async fn order_is_stable_across_requests() {
        let (_, _, first) = fetch().await;
        let (_, _, second) = fetch().await;
        assert_eq!(first, second);
        assert_eq!(strategy_heroes(), strategy_heroes());
    }

    #[tokio::test]
    async fn known_rows_match_the_table() {
        let (_, _, body) = fetch().await;
        let find = |id: &str| {
            data(&body)
                .iter()
                .find(|h| h["id"] == id)
                .unwrap_or_else(|| panic!("missing {id}"))
                .clone()
        };
        let sombra = find("sombra");
        assert_eq!(sombra["name"], "Sombra");
        assert_eq!(
            sombra["role"],
            scuffed_types::role_for_hero_name("Sombra")
                .unwrap()
                .to_string()
                .to_lowercase()
                .as_str()
        );
        assert_eq!(find("wrecking-ball")["role"], "tank");
        assert_eq!(find("ana")["role"], "support");
    }
}

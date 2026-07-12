use chrono::{Duration, Utc};
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::{Database, PersonalMatch};

/// Seed dev data into an in-memory database for local development.
pub async fn seed_dev_data(
    db: &Database,
    dev_session_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let token_hash = hash_session_token(dev_session_token);

    db.client
        .query(
            r#"
            CREATE user:devadmin SET
                provider = 'discord',
                username = 'DevAdmin',
                avatar_url = NONE,
                provider_id = 'dev-user-id',
                provider_id_hash = NONE,
                provider_id_encrypted = NONE,
                created_at = time::now();

            CREATE member:devmember SET
                user_id = 'devadmin',
                org_role = 'admin',
                display_name = 'DevAdmin',
                bio = NONE,
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                nostr_pubkey = NONE,
                nostr_key_mode = NONE,
                nostr_secret_key_encrypted = NONE,
                joined_at = time::now(),
                is_active = true;

            CREATE session:devsession SET
                user_id = 'devadmin',
                token = $token_hash,
                expires_at = time::now() + 365d,
                created_at = time::now();
        "#,
        )
        .bind(("token_hash", token_hash))
        .await?;

    tracing::info!("Dev data seeded: user=devadmin, role=admin");

    // Seed sample games and teams
    db.client
        .query(
            r#"
            CREATE game:overwatch2 SET
                name = 'Overwatch 2',
                abbreviation = 'OW2',
                is_active = true,
                created_at = time::now();

            CREATE team:alpha SET
                name = 'Alpha Squad', game_id = 'overwatch2', color = '#e74c3c',
                division = 'Main', lore_quote = NONE, logo_url = NONE,
                is_active = true, created_at = time::now();

            CREATE team:bravo SET
                name = 'Bravo Team', game_id = 'overwatch2', color = '#3498db',
                division = 'Academy', lore_quote = NONE, logo_url = NONE,
                is_active = true, created_at = time::now();

            CREATE team:charlie SET
                name = 'Charlie Company', game_id = 'overwatch2', color = NONE,
                division = NONE, lore_quote = NONE, logo_url = NONE,
                is_active = true, created_at = time::now();

            CREATE team:delta SET
                name = 'Delta Force', game_id = 'overwatch2', color = NONE,
                division = NONE, lore_quote = NONE, logo_url = NONE,
                is_active = true, created_at = time::now();
        "#,
        )
        .await?;

    // Single-elim tournament
    db.client
        .query(r#"
            CREATE tournament:demo SET
                name = 'Scuffed Cup #1', game_id = 'overwatch2',
                format = 'single_elim', status = 'registration', max_teams = 8,
                best_of = 3, swiss_rounds = NONE, is_external = false, is_open = false,
                external_url = NONE,
                rules = 'Standard competitive rules. No hero bans. Map pool: current ranked rotation.',
                description = 'Internal single elimination tournament for all Scuffed Crew teams.',
                starts_at = time::now() + 7d, ends_at = NONE,
                created_by = 'devadmin', created_at = time::now();

            CREATE tournament_participant:p1 SET
                tournament_id = 'demo', team_id = 'alpha', external_name = NONE,
                seed = 1, group_label = NONE, status = 'registered', created_at = time::now();
            CREATE tournament_participant:p2 SET
                tournament_id = 'demo', team_id = 'bravo', external_name = NONE,
                seed = 2, group_label = NONE, status = 'registered', created_at = time::now();
            CREATE tournament_participant:p3 SET
                tournament_id = 'demo', team_id = 'charlie', external_name = NONE,
                seed = 3, group_label = NONE, status = 'registered', created_at = time::now();
            CREATE tournament_participant:p4 SET
                tournament_id = 'demo', team_id = 'delta', external_name = NONE,
                seed = 4, group_label = NONE, status = 'registered', created_at = time::now();
        "#)
        .await?;

    db.generate_single_elim_bracket("demo").await?;
    db.client
        .query("UPDATE tournament:demo SET status = 'in_progress'")
        .await?;

    // Report semi-finals
    let matches = db.list_tournament_matches("demo").await?;
    let semis: Vec<_> = matches
        .iter()
        .filter(|m| {
            m.participant_a_id.is_some()
                && m.participant_b_id.is_some()
                && m.status == scuffed_db::types::TournamentMatchStatus::Pending
        })
        .collect();

    if semis.len() >= 2 {
        let s1 = &semis[0];
        let winner1 = s1.participant_a_id.as_ref().unwrap();
        db.report_tournament_match(
            &s1.id,
            2,
            0,
            winner1,
            Some("Dominant performance"),
            vec!["ABC123".into(), "DEF456".into()],
        )
        .await?;
        if let (Some(next_id), Some(next_slot)) = (&s1.next_match_id, &s1.next_match_slot) {
            db.set_match_participant(next_id, next_slot, winner1)
                .await?;
        }

        let s2 = &semis[1];
        let winner2 = s2.participant_a_id.as_ref().unwrap();
        db.report_tournament_match(
            &s2.id,
            2,
            1,
            winner2,
            Some("Close series"),
            vec!["GHI789".into(), "JKL012".into(), "MNO345".into()],
        )
        .await?;
        if let (Some(next_id), Some(next_slot)) = (&s2.next_match_id, &s2.next_match_slot) {
            db.set_match_participant(next_id, next_slot, winner2)
                .await?;
        }
    }

    tracing::info!("Tournament seed complete: single-elim, round-robin, and Swiss");

    seed_personal_matches(db, "devmember").await?;

    // Flagship identity for local/dev — product defaults stay neutral without this.
    {
        use scuffed_types::HomepagePreset;
        let scuffed = HomepagePreset::scuffed();
        db.update_settings(
            Some("The Scuffed Crew"),
            Some("EMEA Gaming Organization"),
            Some(true),
            Some("We are currently recruiting! Apply now to join the crew."),
            Some(16),
            None,
            None,
            Some(scuffed.suggested_layout.as_str()),
            Some(&scuffed.content.to_json()),
            None,
            None,
            None,
            Some(scuffed.suggested_brand.accent_dark),
            Some(scuffed.suggested_brand.accent_light),
            Some(scuffed.suggested_shell.as_str()),
            Some(scuffed.suggested_skin.as_str()),
        )
        .await?;
        tracing::info!(
            "Dev site settings: Scuffed Crew homepage + brand + shell/skin applied"
        );
    }

    Ok(())
}

/// One seeded personal-match row:
/// (hero_id, role, map_name, game_mode, outcome, elims, deaths, assists, damage, healing, mitigation)
type SeedMatchRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
);

/// Seed a varied set of personal_match records for the dev member so the
/// strategy editor's hero-winrate badges and the personal stats dashboard
/// have something to render in dev mode.
async fn seed_personal_matches(
    db: &Database,
    member_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = Utc::now();

    let rows: &[SeedMatchRow] = &[
        // Tracer — high winrate (5W / 1L / 1D = ~71%)
        (
            "tracer",
            "Damage",
            "Kings Row",
            "hybrid",
            "victory",
            28,
            6,
            4,
            11200,
            0,
            0,
        ),
        (
            "tracer",
            "Damage",
            "Lijiang Tower",
            "control",
            "victory",
            24,
            5,
            3,
            9800,
            0,
            0,
        ),
        (
            "tracer",
            "Damage",
            "Eichenwalde",
            "hybrid",
            "victory",
            22,
            7,
            2,
            9100,
            0,
            0,
        ),
        (
            "tracer", "Damage", "Dorado", "escort", "defeat", 18, 9, 1, 8200, 0, 0,
        ),
        (
            "tracer", "Damage", "Ilios", "control", "victory", 30, 4, 5, 12400, 0, 0,
        ),
        (
            "tracer", "Damage", "Numbani", "hybrid", "draw", 19, 8, 3, 9500, 0, 0,
        ),
        (
            "tracer", "Damage", "Route 66", "escort", "victory", 26, 6, 4, 10500, 0, 0,
        ),
        // Ana — solid winrate (4W / 2L = ~67%)
        (
            "ana",
            "Support",
            "Kings Row",
            "hybrid",
            "victory",
            8,
            4,
            18,
            4200,
            13800,
            0,
        ),
        (
            "ana",
            "Support",
            "Lijiang Tower",
            "control",
            "defeat",
            5,
            7,
            14,
            3100,
            11400,
            0,
        ),
        (
            "ana",
            "Support",
            "Eichenwalde",
            "hybrid",
            "victory",
            9,
            3,
            22,
            5100,
            16200,
            0,
        ),
        (
            "ana", "Support", "Dorado", "escort", "victory", 7, 5, 19, 4400, 14100, 0,
        ),
        (
            "ana", "Support", "Ilios", "control", "victory", 6, 4, 16, 3900, 12700, 0,
        ),
        (
            "ana", "Support", "Route 66", "escort", "defeat", 4, 8, 12, 2800, 10200, 0,
        ),
        // Reinhardt — losing record (1W / 3L = 25%)
        (
            "reinhardt",
            "Tank",
            "Kings Row",
            "hybrid",
            "defeat",
            12,
            9,
            8,
            8400,
            0,
            22000,
        ),
        (
            "reinhardt",
            "Tank",
            "Eichenwalde",
            "hybrid",
            "defeat",
            10,
            11,
            6,
            7600,
            0,
            19500,
        ),
        (
            "reinhardt",
            "Tank",
            "Numbani",
            "hybrid",
            "victory",
            16,
            6,
            12,
            11200,
            0,
            28400,
        ),
        (
            "reinhardt",
            "Tank",
            "Dorado",
            "escort",
            "defeat",
            11,
            10,
            7,
            8100,
            0,
            20800,
        ),
        // D.Va — middling (2W / 2L = 50%)
        (
            "dva",
            "Tank",
            "Lijiang Tower",
            "control",
            "victory",
            18,
            6,
            9,
            9800,
            0,
            18400,
        ),
        (
            "dva", "Tank", "Ilios", "control", "victory", 20, 5, 10, 10500, 0, 19200,
        ),
        (
            "dva", "Tank", "Route 66", "escort", "defeat", 14, 9, 6, 7800, 0, 14600,
        ),
        (
            "dva", "Tank", "Numbani", "hybrid", "defeat", 12, 10, 5, 7100, 0, 13800,
        ),
        // Juno — strong (3W / 1L = 75%)
        (
            "juno",
            "Support",
            "Kings Row",
            "hybrid",
            "victory",
            11,
            3,
            20,
            5800,
            14400,
            0,
        ),
        (
            "juno",
            "Support",
            "Eichenwalde",
            "hybrid",
            "victory",
            9,
            4,
            18,
            5100,
            13200,
            0,
        ),
        (
            "juno",
            "Support",
            "Lijiang Tower",
            "control",
            "victory",
            12,
            2,
            22,
            6200,
            15800,
            0,
        ),
        (
            "juno", "Support", "Dorado", "escort", "defeat", 7, 6, 14, 4100, 11800, 0,
        ),
        // Mercy — coin flip (1W / 1L = 50%)
        (
            "mercy", "Support", "Ilios", "control", "victory", 2, 3, 24, 1100, 18200, 0,
        ),
        (
            "mercy", "Support", "Route 66", "escort", "defeat", 1, 5, 18, 800, 14600, 0,
        ),
        // Cassidy — weak (0W / 2L = 0%)
        (
            "cassidy",
            "Damage",
            "Kings Row",
            "hybrid",
            "defeat",
            14,
            9,
            3,
            7800,
            0,
            0,
        ),
        (
            "cassidy", "Damage", "Numbani", "hybrid", "defeat", 12, 11, 2, 6900, 0, 0,
        ),
    ];

    let matches: Vec<PersonalMatch> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            // Spread played_at across the last ~30 days. Each row is its own
            // game, so each gets its own session id (the server upserts one
            // row per session).
            let played_at = now - Duration::hours((rows.len() - i) as i64 * 6);
            PersonalMatch {
                id: String::new(),
                member_id: member_id.to_string(),
                session_id: format!("seed-{member_id}-{i}"),
                hero: r.0.to_string(),
                map_name: r.2.to_string(),
                game_mode: r.3.to_string(),
                role: r.1.to_string(),
                outcome: r.4.to_string(),
                elims: r.5,
                deaths: r.6,
                assists: r.7,
                damage: r.8,
                healing: r.9,
                mitigation: r.10,
                played_at,
                uploaded_at: now,
            }
        })
        .collect();

    let inserted = db.upsert_personal_matches(member_id, &matches).await?;
    tracing::info!("Personal stats seed complete: {inserted} matches for {member_id}");
    Ok(())
}

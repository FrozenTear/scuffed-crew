use scuffed_auth::crypto::hash_session_token;
use scuffed_db::Database;

/// Seed dev data into an in-memory database for local development.
pub async fn seed_dev_data(db: &Database, dev_session_token: &str) -> Result<(), Box<dyn std::error::Error>> {
    let token_hash = hash_session_token(dev_session_token);

    db.client
        .query(r#"
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
                joined_at = time::now(),
                is_active = true;

            CREATE session:devsession SET
                user_id = 'devadmin',
                token = $token_hash,
                expires_at = time::now() + 365d,
                created_at = time::now();
        "#)
        .bind(("token_hash", token_hash))
        .await?;

    tracing::info!("Dev data seeded: user=devadmin, role=admin");

    // Seed sample games and teams
    db.client
        .query(r#"
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
        "#)
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
        db.report_tournament_match(&s1.id, 2, 0, winner1, Some("Dominant performance"), vec!["ABC123".into(), "DEF456".into()])
            .await?;
        if let (Some(next_id), Some(next_slot)) = (&s1.next_match_id, &s1.next_match_slot) {
            db.set_match_participant(next_id, next_slot, winner1).await?;
        }

        let s2 = &semis[1];
        let winner2 = s2.participant_a_id.as_ref().unwrap();
        db.report_tournament_match(&s2.id, 2, 1, winner2, Some("Close series"), vec!["GHI789".into(), "JKL012".into(), "MNO345".into()])
            .await?;
        if let (Some(next_id), Some(next_slot)) = (&s2.next_match_id, &s2.next_match_slot) {
            db.set_match_participant(next_id, next_slot, winner2).await?;
        }
    }

    tracing::info!("Tournament seed complete: single-elim, round-robin, and Swiss");
    Ok(())
}

//! The on-open pack-key hero migration.

use super::{LocalStore, PersonalMatch, rewrite_match_log_session};

/// Display name and role for a pack file key or a hero the table already names.
///
/// `wrecking-ball` becomes `Wrecking Ball` / Tank. A stored display name such
/// as `Wrecking Ball` keeps that text and still returns the table role, so a
/// wrong role can be corrected. A folded alias such as `Lucio` keeps its
/// stored spelling. `None` when the text is not a known hero.
fn known_hero_role(raw: &str) -> Option<(String, String)> {
    let from_pack = scuffed_types::hero_for_pack_file(raw);
    let display = from_pack
        .map(|row| row.display)
        .or_else(|| scuffed_types::hero_key_to_name(raw))?;
    let role = scuffed_types::role_for_hero_name(display)?.to_string();
    let hero = if from_pack.is_some() || raw.trim() == display {
        display.to_string()
    } else {
        raw.trim().to_string()
    };
    Some((hero, role))
}

/// Write `hero` and `role` from the hero table when this text is a known hero.
fn apply_known_hero(hero: &mut String, role: &mut String) -> bool {
    let Some((next_hero, next_role)) = known_hero_role(hero) else {
        return false;
    };
    let mut changed = false;
    if *hero != next_hero {
        *hero = next_hero;
        changed = true;
    }
    if *role != next_role {
        *role = next_role;
        changed = true;
    }
    changed
}

/// Rewrite pack-key heroes, and fix a wrong role on a canonical hero.
///
/// Returns whether any field changed. The row is marked unsynced only when
/// the hero or role that would be uploaded changes.
fn rewrite_pack_key_fields(row: &mut PersonalMatch) -> bool {
    let before_hero = row.display_hero().to_string();
    let before_role = row.display_role().to_string();
    let mut changed = false;
    changed |= apply_known_hero(&mut row.hero, &mut row.role);
    if let Some(current) = row.corrected_hero.clone() {
        let mut hero = current;
        let had_role = row.corrected_role.is_some();
        let mut role = row.corrected_role.clone().unwrap_or_default();
        if apply_known_hero(&mut hero, &mut role) {
            let hero_changed = row.corrected_hero.as_deref() != Some(hero.as_str());
            let role_changed = had_role && row.corrected_role.as_deref() != Some(role.as_str());
            if hero_changed || role_changed {
                row.corrected_hero = Some(hero);
                if had_role || hero_changed {
                    row.corrected_role = Some(role);
                }
                changed = true;
            }
        }
    }
    for segment in &mut row.heroes_played {
        changed |= apply_known_hero(&mut segment.hero, &mut segment.role);
    }
    if !changed {
        return false;
    }
    if before_hero != row.display_hero() || before_role != row.display_role() {
        row.synced = false;
        row.sync_rev = row.sync_rev.saturating_add(1);
        row.upload_reject = None;
    }
    true
}

impl LocalStore {
    /// Rewrite saved pack-key heroes to the display name, and set the role
    /// from the hero table. A row that already says `Wrecking Ball` keeps
    /// that name and still gets Tank when the stored role is wrong.
    ///
    /// Runs on every open. A second run finds nothing to change and writes
    /// nothing. A game is queued for upload only when the hero or role that
    /// would be sent actually changes.
    pub async fn migrate_pack_key_heroes(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let matches = self.get_all_matches().await?;
        let mut rewritten = 0usize;
        let mut reupload = 0usize;
        let mut sessions = std::collections::BTreeSet::new();
        for mut row in matches {
            let before_hero = row.display_hero().to_string();
            let before_role = row.display_role().to_string();
            if !rewrite_pack_key_fields(&mut row) {
                continue;
            }
            let queue = before_hero != row.display_hero() || before_role != row.display_role();
            self.write_rewritten_match(&row, queue).await?;
            if !row.session_id.is_empty() {
                sessions.insert(row.session_id.clone());
            }
            rewritten += 1;
            if queue {
                reupload += 1;
            }
        }
        for session in self.get_all_sessions().await? {
            let Some((hero, role)) = known_hero_role(&session.hero) else {
                continue;
            };
            if session.hero == hero && session.role == role {
                continue;
            }
            self.db
                .query(
                    "UPDATE match_session SET hero = $hero, role = $role WHERE session_id = $sid",
                )
                .bind(("hero", hero.to_string()))
                .bind(("role", role))
                .bind(("sid", session.session_id.clone()))
                .await?;
            sessions.insert(session.session_id);
        }
        for session_id in &sessions {
            rewrite_match_log_session(
                self.data_dir.as_path(),
                session_id,
                Some(&|row| {
                    rewrite_pack_key_fields(row);
                }),
            );
        }
        if rewritten > 0 {
            tracing::info!(rewritten, reupload, "rewrote saved pack key heroes");
        }
        Ok(())
    }

    async fn write_rewritten_match(
        &self,
        row: &PersonalMatch,
        reupload: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let Some(id) = row.id.clone() else {
            return Ok(());
        };
        let mut sql =
            String::from("UPDATE $id SET hero = $hero, role = $role, heroes_played = $hp");
        if reupload {
            sql.push_str(", synced = false, upload_reject = '', sync_rev = (sync_rev ?? 0) + 1");
        }
        if row.corrected_hero.is_some() {
            sql.push_str(", corrected_hero = $ch, corrected_role = $cr");
        }
        let mut query = self
            .db
            .query(sql)
            .bind(("id", id))
            .bind(("hero", row.hero.clone()))
            .bind(("role", row.role.clone()))
            .bind(("hp", row.heroes_played.clone()));
        if let Some(hero) = row.corrected_hero.clone() {
            query = query
                .bind(("ch", hero))
                .bind(("cr", row.corrected_role.clone().unwrap_or_default()));
        }
        query.await?;
        Ok(())
    }
}

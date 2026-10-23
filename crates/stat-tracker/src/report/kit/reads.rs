//! `reads.json`: the reads of one accepted capture, as the report manifest
//! carries them ([`KitReads`]).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::super::blank::{self, TeamLayout};
use crate::reader_apply::ReaderRead;

/// One read in `reads.json`, the same shape as a manifest read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KitRead {
    pub value: Option<String>,
    /// The new reader's confidence for this read, 0 to 1
    /// ([`KitReads::with_confidences`]). `null` under ocr-v1, for a stat
    /// the capture gate replaced, and for a result filled from the stored
    /// game ([`KitReads::fill_result`]).
    pub confidence: Option<f64>,
    /// The read before the capture gate (spec section 2). `reader =
    /// "ocr-v1"`: the Tesseract read. `reader = "new"`: Tesseract reads no
    /// stat cell, so for the six stats this is the digit read (for a held
    /// cell, the stored value). [`KitReads::recognizer`] tells the two
    /// apart. The wire name is unchanged.
    pub ocr_v1: Option<String>,
    pub suspect: bool,
}

/// `reads.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KitReads {
    pub captured_at: DateTime<Utc>,
    pub width: u32,
    pub height: u32,
    /// `null` when the board was blanked with both layouts.
    pub team_size: Option<u32>,
    pub map: Option<String>,
    pub mode: Option<String>,
    pub result: Option<String>,
    /// The stored row's recognizer tag: the matcher id (`cv-v5`) for a
    /// capture whose stats came from the digit reader, `ocr-v1` for a
    /// Tesseract capture. Under `reader = "new"` every stored capture has
    /// at least four digit-read stats, so this is always the matcher id
    /// there, and the stats' `ocr_v1` are then digit reads too.
    pub recognizer: String,
    /// [`blank::BLANK_TABLE_VERSION`] the scoreboard was blanked with. A kit
    /// from another table is not reported (`load` drops its scoreboard).
    #[serde(default)]
    pub blank_table: u32,
    /// SHA-256 (hex) of the `scoreboard.png` written together with these
    /// reads. The daemon replaces the two files one after the other, so a
    /// reader in another process can land between the two renames; `load`
    /// only returns a pair whose hash matches.
    #[serde(default)]
    pub scoreboard_sha256: String,
    /// Keyed by the manifest read keys (`mode`, `result`, `hero`, `e`, `a`,
    /// `d`, `dmg`, `h`, `mit`).
    pub reads: BTreeMap<String, KitRead>,
}

impl KitReads {
    /// Reads of one accepted capture. `before` is the match as it was read,
    /// before the capture gate changed a stat (the `ocr_v1` read: the
    /// Tesseract pass under `reader = "ocr-v1"`, the digit reads under
    /// `reader = "new"`); `after` is what was stored.
    pub fn from_capture(
        before: &crate::storage::PersonalMatch,
        after: &crate::storage::PersonalMatch,
        width: u32,
        height: u32,
        layout: TeamLayout,
        captured_at: DateTime<Utc>,
    ) -> Self {
        let suspects = crate::suspect::upload_suspect_fields(&after.suspect_fields);
        let text = |s: &str| {
            let s = s.trim();
            (!s.is_empty()).then(|| s.chars().take(64).collect::<String>())
        };
        let mut reads = BTreeMap::new();
        // Spec section 2: a read is suspect when the tracker flags it, when
        // its value is null, or (stats) when the stored value and the
        // Tesseract read disagree.
        let mut put = |key: &str, value: Option<String>, ocr: Option<String>, disagree: bool| {
            let suspect = suspects.iter().any(|f| f == key) || value.is_none() || disagree;
            reads.insert(
                key.to_string(),
                KitRead {
                    value,
                    confidence: None,
                    ocr_v1: ocr,
                    suspect,
                },
            );
        };
        put(
            "mode",
            text(&after.game_mode),
            text(&before.game_mode),
            false,
        );
        put("result", text(&after.outcome), text(&before.outcome), false);
        put("hero", text(&after.hero), text(&before.hero), false);
        for (key, a, b) in [
            ("e", after.elims, before.elims),
            ("a", after.assists, before.assists),
            ("d", after.deaths, before.deaths),
            ("dmg", after.damage, before.damage),
            ("h", after.healing, before.healing),
            ("mit", after.mitigation, before.mitigation),
        ] {
            put(key, Some(a.to_string()), Some(b.to_string()), a != b);
        }
        Self {
            captured_at,
            width,
            height,
            team_size: layout.team_size(),
            map: text(&after.map_name),
            mode: text(&after.game_mode),
            result: text(&after.outcome),
            blank_table: blank::BLANK_TABLE_VERSION,
            scoreboard_sha256: String::new(),
            recognizer: if after.recognizer.is_empty() {
                "ocr-v1".to_string()
            } else {
                after.recognizer.clone()
            },
            reads,
        }
    }
}

/// True unless `result` is a decided outcome (victory, defeat, draw).
pub fn result_is_unknown(result: &str) -> bool {
    let r = result.trim();
    !["victory", "defeat", "draw"]
        .iter()
        .any(|o| r.eq_ignore_ascii_case(o))
}

impl KitReads {
    /// Fill each read's `confidence` from the active reader's (keyed by read
    /// key). A stat the capture gate replaced (the stored value is not the
    /// reader's own read) gets none: the confidence belongs to the read, not
    /// to the stored value. A reader read that disagrees with Tesseract
    /// (`ocr_v1`) keeps its confidence. NaN and infinite give none; other
    /// values are clamped to 0..1.
    pub fn with_confidences(mut self, confidences: &BTreeMap<String, ReaderRead>) -> Self {
        for (key, read) in self.reads.iter_mut() {
            let Some(r) = confidences.get(key) else {
                continue;
            };
            let c = r.confidence;
            let gated = matches!(key.as_str(), "e" | "a" | "d" | "dmg" | "h" | "mit")
                && (r.value.is_none() || read.value != r.value);
            if !gated && c.is_finite() {
                read.confidence = Some(c.clamp(0.0, 1.0));
            }
        }
        self
    }

    /// The game's result as stored when the game closed, for a kit whose
    /// Tab capture came before the result was known. The Tab read stays in
    /// `ocr_v1`. A known kit result is never replaced.
    pub fn fill_result(&mut self, stored_outcome: &str) {
        let known = |s: &str| !result_is_unknown(s);
        if !known(stored_outcome) || self.result.as_deref().is_some_and(known) {
            return;
        }
        let outcome = stored_outcome.trim().chars().take(64).collect::<String>();
        self.result = Some(outcome.clone());
        let read = self.reads.entry("result".into()).or_insert(KitRead {
            value: None,
            confidence: None,
            ocr_v1: None,
            suspect: false,
        });
        read.value = Some(outcome);
        read.confidence = None;
        read.suspect = false;
    }
}

use serde::{Deserialize, Serialize};

use crate::strategy::{GameMode, HeroRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hero {
    // Tank (14)
    DVa,
    Domina,
    Doomfist,
    Hazard,
    JunkerQueen,
    Mauga,
    Orisa,
    Ramattra,
    Reinhardt,
    Roadhog,
    Sigma,
    Winston,
    WreckingBall,
    Zarya,

    // Damage (23)
    Anran,
    Ashe,
    Bastion,
    Cassidy,
    Echo,
    Emre,
    Freja,
    Genji,
    Hanzo,
    Junkrat,
    Mei,
    Pharah,
    Reaper,
    Sierra,
    Sojourn,
    Soldier76,
    Sombra,
    Symmetra,
    Torbjorn,
    Tracer,
    Vendetta,
    Venture,
    Widowmaker,

    // Support (13)
    Ana,
    Baptiste,
    Brigitte,
    Illari,
    Juno,
    Kiriko,
    Lifeweaver,
    Lucio,
    Mercy,
    Mizuki,
    Moira,
    Wuyang,
    Zenyatta,
}

impl Hero {
    pub fn role(&self) -> HeroRole {
        match self {
            Self::DVa
            | Self::Domina
            | Self::Doomfist
            | Self::Hazard
            | Self::JunkerQueen
            | Self::Mauga
            | Self::Orisa
            | Self::Ramattra
            | Self::Reinhardt
            | Self::Roadhog
            | Self::Sigma
            | Self::Winston
            | Self::WreckingBall
            | Self::Zarya => HeroRole::Tank,

            Self::Anran
            | Self::Ashe
            | Self::Bastion
            | Self::Cassidy
            | Self::Echo
            | Self::Emre
            | Self::Freja
            | Self::Genji
            | Self::Hanzo
            | Self::Junkrat
            | Self::Mei
            | Self::Pharah
            | Self::Reaper
            | Self::Sierra
            | Self::Sojourn
            | Self::Soldier76
            | Self::Sombra
            | Self::Symmetra
            | Self::Torbjorn
            | Self::Tracer
            | Self::Vendetta
            | Self::Venture
            | Self::Widowmaker => HeroRole::Damage,

            Self::Ana
            | Self::Baptiste
            | Self::Brigitte
            | Self::Illari
            | Self::Juno
            | Self::Kiriko
            | Self::Lifeweaver
            | Self::Lucio
            | Self::Mercy
            | Self::Mizuki
            | Self::Moira
            | Self::Wuyang
            | Self::Zenyatta => HeroRole::Support,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::DVa => "D.Va",
            Self::Domina => "Domina",
            Self::Doomfist => "Doomfist",
            Self::Hazard => "Hazard",
            Self::JunkerQueen => "Junker Queen",
            Self::Mauga => "Mauga",
            Self::Orisa => "Orisa",
            Self::Ramattra => "Ramattra",
            Self::Reinhardt => "Reinhardt",
            Self::Roadhog => "Roadhog",
            Self::Sigma => "Sigma",
            Self::Winston => "Winston",
            Self::WreckingBall => "Wrecking Ball",
            Self::Zarya => "Zarya",
            Self::Anran => "Anran",
            Self::Ashe => "Ashe",
            Self::Bastion => "Bastion",
            Self::Cassidy => "Cassidy",
            Self::Echo => "Echo",
            Self::Emre => "Emre",
            Self::Freja => "Freja",
            Self::Genji => "Genji",
            Self::Hanzo => "Hanzo",
            Self::Junkrat => "Junkrat",
            Self::Mei => "Mei",
            Self::Pharah => "Pharah",
            Self::Reaper => "Reaper",
            Self::Sierra => "Sierra",
            Self::Sojourn => "Sojourn",
            Self::Soldier76 => "Soldier: 76",
            Self::Sombra => "Sombra",
            Self::Symmetra => "Symmetra",
            Self::Torbjorn => "Torbjörn",
            Self::Tracer => "Tracer",
            Self::Vendetta => "Vendetta",
            Self::Venture => "Venture",
            Self::Widowmaker => "Widowmaker",
            Self::Ana => "Ana",
            Self::Baptiste => "Baptiste",
            Self::Brigitte => "Brigitte",
            Self::Illari => "Illari",
            Self::Juno => "Juno",
            Self::Kiriko => "Kiriko",
            Self::Lifeweaver => "Lifeweaver",
            Self::Lucio => "Lúcio",
            Self::Mercy => "Mercy",
            Self::Mizuki => "Mizuki",
            Self::Moira => "Moira",
            Self::Wuyang => "Wuyang",
            Self::Zenyatta => "Zenyatta",
        }
    }
}

impl std::fmt::Display for Hero {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapName {
    // Escort
    CircuitRoyal,
    Dorado,
    Havana,
    Junkertown,
    Rialto,
    Route66,
    ShambaliMonastery,
    WatchpointGibraltar,

    // Hybrid
    BlizzardWorld,
    Eichenwalde,
    Hollywood,
    KingsRow,
    Midtown,
    NeonJunction,
    Numbani,
    Paraiso,

    // Control
    AntarcticPeninsula,
    Busan,
    Ilios,
    LijangTower,
    Nepal,
    Oasis,
    Samoa,

    // Push
    Colosseo,
    Esperanca,
    NewQueenStreet,
    Runasapi,

    // Flashpoint
    Aatlis,
    NewJunkCity,
    Suravasa,

    // Clash
    Hanaoka,
    ThroneOfAnubis,
}

impl MapName {
    pub fn game_mode(&self) -> GameMode {
        match self {
            Self::CircuitRoyal
            | Self::Dorado
            | Self::Havana
            | Self::Junkertown
            | Self::Rialto
            | Self::Route66
            | Self::ShambaliMonastery
            | Self::WatchpointGibraltar => GameMode::Escort,

            Self::BlizzardWorld
            | Self::Eichenwalde
            | Self::Hollywood
            | Self::KingsRow
            | Self::Midtown
            | Self::NeonJunction
            | Self::Numbani
            | Self::Paraiso => GameMode::Hybrid,

            Self::AntarcticPeninsula
            | Self::Busan
            | Self::Ilios
            | Self::LijangTower
            | Self::Nepal
            | Self::Oasis
            | Self::Samoa => GameMode::Control,

            Self::Colosseo | Self::Esperanca | Self::NewQueenStreet | Self::Runasapi => {
                GameMode::Push
            }

            Self::Aatlis | Self::NewJunkCity | Self::Suravasa => GameMode::Flashpoint,

            Self::Hanaoka | Self::ThroneOfAnubis => GameMode::Clash,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::CircuitRoyal => "Circuit Royal",
            Self::Dorado => "Dorado",
            Self::Havana => "Havana",
            Self::Junkertown => "Junkertown",
            Self::Rialto => "Rialto",
            Self::Route66 => "Route 66",
            Self::ShambaliMonastery => "Shambali Monastery",
            Self::WatchpointGibraltar => "Watchpoint: Gibraltar",
            Self::BlizzardWorld => "Blizzard World",
            Self::Eichenwalde => "Eichenwalde",
            Self::Hollywood => "Hollywood",
            Self::KingsRow => "King's Row",
            Self::Midtown => "Midtown",
            Self::NeonJunction => "Neon Junction",
            Self::Numbani => "Numbani",
            Self::Paraiso => "Paraíso",
            Self::AntarcticPeninsula => "Antarctic Peninsula",
            Self::Busan => "Busan",
            Self::Ilios => "Ilios",
            Self::LijangTower => "Lijiang Tower",
            Self::Nepal => "Nepal",
            Self::Oasis => "Oasis",
            Self::Samoa => "Samoa",
            Self::Colosseo => "Colosseo",
            Self::Esperanca => "Esperança",
            Self::NewQueenStreet => "New Queen Street",
            Self::Runasapi => "Runasapi",
            Self::Aatlis => "Aatlis",
            Self::NewJunkCity => "New Junk City",
            Self::Suravasa => "Suravasa",
            Self::Hanaoka => "Hanaoka",
            Self::ThroneOfAnubis => "Throne of Anubis",
        }
    }

    /// Maps-tab mode bucket for a stored/OCR map string. Unknown names are
    /// `"Other"` (the UI catch-all), not a `GameMode` variant.
    pub fn game_mode_label(name: &str) -> &'static str {
        match name.parse::<Self>() {
            Ok(map) => match map.game_mode() {
                GameMode::Escort => "Escort",
                GameMode::Hybrid => "Hybrid",
                GameMode::Control => "Control",
                GameMode::Push => "Push",
                GameMode::Flashpoint => "Flashpoint",
                GameMode::Clash => "Clash",
                GameMode::PayloadRace => "Payload Race",
                GameMode::Assault => "Assault",
            },
            Err(()) => "Other",
        }
    }
}

impl std::fmt::Display for MapName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Fold a live/OCR map string to a comparable key: lowercase, strip
/// combining-style punctuation, and drop Portuguese/Spanish accents so
/// `"Paraíso"` / `"Paraiso"` and `"Esperança"` / `"Esperanca"` collide.
fn fold_map_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let mapped = match c {
            'Á' | 'À' | 'Ã' | 'Â' | 'á' | 'à' | 'ã' | 'â' => 'a',
            'É' | 'Ê' | 'é' | 'ê' => 'e',
            'Í' | 'í' => 'i',
            'Ó' | 'Ô' | 'Õ' | 'ó' | 'ô' | 'õ' => 'o',
            'Ú' | 'Ü' | 'ú' | 'ü' => 'u',
            'Ç' | 'ç' => 'c',
            'Ñ' | 'ñ' => 'n',
            '\'' => continue,
            '_' | '-' | ':' => ' ',
            other => other,
        };
        for lower in mapped.to_lowercase() {
            if lower.is_whitespace() {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            } else {
                out.push(lower);
            }
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

impl std::str::FromStr for MapName {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match fold_map_key(s).as_str() {
            "circuit royal" | "circuitroyal" => Ok(Self::CircuitRoyal),
            "dorado" => Ok(Self::Dorado),
            "havana" => Ok(Self::Havana),
            "junkertown" => Ok(Self::Junkertown),
            "rialto" => Ok(Self::Rialto),
            "route 66" | "route66" => Ok(Self::Route66),
            "shambali monastery" | "shambali" => Ok(Self::ShambaliMonastery),
            "watchpoint gibraltar" | "watchpointgibraltar" | "watchpoint" => {
                Ok(Self::WatchpointGibraltar)
            }
            "blizzard world" | "blizzardworld" => Ok(Self::BlizzardWorld),
            "eichenwalde" => Ok(Self::Eichenwalde),
            "hollywood" => Ok(Self::Hollywood),
            "kings row" | "kingsrow" => Ok(Self::KingsRow),
            "midtown" => Ok(Self::Midtown),
            "neon junction" | "neonjunction" => Ok(Self::NeonJunction),
            "numbani" => Ok(Self::Numbani),
            "paraiso" => Ok(Self::Paraiso),
            "antarctic peninsula" | "antarcticpeninsula" => Ok(Self::AntarcticPeninsula),
            "busan" => Ok(Self::Busan),
            "ilios" => Ok(Self::Ilios),
            "lijiang tower" | "lijang tower" | "lijiangtower" | "lijangtower" | "lijiang" => {
                Ok(Self::LijangTower)
            }
            "nepal" => Ok(Self::Nepal),
            "oasis" => Ok(Self::Oasis),
            "samoa" => Ok(Self::Samoa),
            "colosseo" => Ok(Self::Colosseo),
            "esperanca" => Ok(Self::Esperanca),
            "new queen street" | "newqueenstreet" => Ok(Self::NewQueenStreet),
            "runasapi" => Ok(Self::Runasapi),
            "aatlis" => Ok(Self::Aatlis),
            "new junk city" | "newjunkcity" => Ok(Self::NewJunkCity),
            "suravasa" => Ok(Self::Suravasa),
            "hanaoka" => Ok(Self::Hanaoka),
            "throne of anubis" | "throneofanubis" => Ok(Self::ThroneOfAnubis),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    Win,
    Loss,
    Draw,
}

impl std::fmt::Display for MatchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Win => write!(f, "Win"),
            Self::Loss => write!(f, "Loss"),
            Self::Draw => write!(f, "Draw"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// Live Maps-tab names (accented + unaccented) plus Neon Junction.
    /// These six strings are the ones that used to land in Other.
    #[test]
    fn six_map_string_forms_classify_mode() {
        let cases = [
            ("Neon Junction", "Hybrid", MapName::NeonJunction),
            ("Paraíso", "Hybrid", MapName::Paraiso),
            ("Paraiso", "Hybrid", MapName::Paraiso),
            ("Esperança", "Push", MapName::Esperanca),
            ("Esperanca", "Push", MapName::Esperanca),
            ("neon junction", "Hybrid", MapName::NeonJunction),
        ];
        for (name, mode, parsed) in cases {
            assert_eq!(
                MapName::from_str(name),
                Ok(parsed),
                "FromStr failed for {name:?}"
            );
            assert_eq!(
                parsed.game_mode(),
                match mode {
                    "Hybrid" => GameMode::Hybrid,
                    "Push" => GameMode::Push,
                    other => panic!("unexpected mode fixture {other}"),
                },
                "game_mode() wrong for {name:?}"
            );
            assert_eq!(
                MapName::game_mode_label(name),
                mode,
                "game_mode_label failed for {name:?}"
            );
        }
    }
}

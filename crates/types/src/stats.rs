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

    // Damage (22)
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
    Symmetra,
    Torbjorn,
    Tracer,
    Vendetta,
    Venture,
    Widowmaker,

    // Support (15)
    Ana,
    Baptiste,
    Brigitte,
    Doctrine,
    Illari,
    Juno,
    Kiriko,
    Lifeweaver,
    Lucio,
    Mercy,
    Mizuki,
    Moira,
    Sombra,
    Wuyang,
    Zenyatta,
}

macro_rules! hero_all {
    ($($variant:ident),* $(,)?) => {
        /// Every `Hero` variant, in enum order.
        ///
        /// [`_hero_all_exhaustive`] matches the same variants with no wildcard,
        /// so a new `Hero` fails to compile until it is added here. A repeated
        /// entry is an unreachable pattern.
        pub const ALL: &'static [Hero] = &[$(Hero::$variant),*];

        const fn _hero_all_exhaustive(hero: Hero) {
            match hero {
                $(Hero::$variant => {}),*
            }
        }
    };
}

impl Hero {
    hero_all! {
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
        Symmetra,
        Torbjorn,
        Tracer,
        Vendetta,
        Venture,
        Widowmaker,
        Ana,
        Baptiste,
        Brigitte,
        Doctrine,
        Illari,
        Juno,
        Kiriko,
        Lifeweaver,
        Lucio,
        Mercy,
        Mizuki,
        Moira,
        Sombra,
        Wuyang,
        Zenyatta,
    }

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
            | Self::Symmetra
            | Self::Torbjorn
            | Self::Tracer
            | Self::Vendetta
            | Self::Venture
            | Self::Widowmaker => HeroRole::Damage,

            Self::Ana
            | Self::Baptiste
            | Self::Brigitte
            | Self::Doctrine
            | Self::Illari
            | Self::Juno
            | Self::Kiriko
            | Self::Lifeweaver
            | Self::Lucio
            | Self::Mercy
            | Self::Mizuki
            | Self::Moira
            | Self::Sombra
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
            Self::Symmetra => "Symmetra",
            Self::Torbjorn => "Torbjörn",
            Self::Tracer => "Tracer",
            Self::Vendetta => "Vendetta",
            Self::Venture => "Venture",
            Self::Widowmaker => "Widowmaker",
            Self::Ana => "Ana",
            Self::Baptiste => "Baptiste",
            Self::Brigitte => "Brigitte",
            Self::Doctrine => "Doctrine",
            Self::Illari => "Illari",
            Self::Juno => "Juno",
            Self::Kiriko => "Kiriko",
            Self::Lifeweaver => "Lifeweaver",
            Self::Lucio => "Lúcio",
            Self::Mercy => "Mercy",
            Self::Mizuki => "Mizuki",
            Self::Moira => "Moira",
            Self::Sombra => "Sombra",
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

macro_rules! map_name_all {
    ($($variant:ident),* $(,)?) => {
        /// Every map variant, in enum order. Callers that used to copy display
        /// names by hand (OCR collision checks) iterate this instead.
        ///
        /// [`_map_name_all_exhaustive`] matches the same variants with no
        /// wildcard, so a new `MapName` fails to compile until it is added here.
        pub const ALL: &[MapName] = &[$(MapName::$variant),*];

        const fn _map_name_all_exhaustive(map: MapName) {
            match map {
                $(MapName::$variant => {}),*
            }
        }
    };
}

impl MapName {
    map_name_all! {
        CircuitRoyal,
        Dorado,
        Havana,
        Junkertown,
        Rialto,
        Route66,
        ShambaliMonastery,
        WatchpointGibraltar,
        BlizzardWorld,
        Eichenwalde,
        Hollywood,
        KingsRow,
        Midtown,
        NeonJunction,
        Numbani,
        Paraiso,
        AntarcticPeninsula,
        Busan,
        Ilios,
        LijangTower,
        Nepal,
        Oasis,
        Samoa,
        Colosseo,
        Esperanca,
        NewQueenStreet,
        Runasapi,
        Aatlis,
        NewJunkCity,
        Suravasa,
        Hanaoka,
        ThroneOfAnubis,
    }

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

    #[test]
    fn hero_all_display_names_are_unique() {
        let mut names: Vec<&str> = Hero::ALL.iter().map(|hero| hero.display_name()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "display names must be unique");
        assert_eq!(Hero::ALL.len(), 51);
    }

    #[test]
    fn all_covers_every_map_and_display_names_round_trip() {
        assert_eq!(MapName::ALL.len(), 32);
        let mut names: Vec<&str> = MapName::ALL.iter().map(|m| m.display_name()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "display names must be unique");
        for map in MapName::ALL {
            assert_eq!(map.display_name().parse::<MapName>().unwrap(), *map);
        }
    }

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

    /// Season 5: Doctrine is Support; Sombra moved Damage → Support.
    /// Serde names stay snake_case (`doctrine`, `sombra`). Roadhog's rework
    /// did not change his role.
    #[test]
    fn season5_doctrine_and_sombra_are_support() {
        assert_eq!(Hero::Doctrine.role(), HeroRole::Support);
        assert_eq!(Hero::Sombra.role(), HeroRole::Support);
        assert_eq!(Hero::Roadhog.role(), HeroRole::Tank);
        assert_eq!(Hero::Doctrine.display_name(), "Doctrine");
        assert_eq!(Hero::Doctrine.to_string(), "Doctrine");
        assert_eq!(Hero::Sombra.to_string(), "Sombra");

        let doctrine: Hero = serde_json::from_str("\"doctrine\"").unwrap();
        assert_eq!(doctrine, Hero::Doctrine);
        assert_eq!(
            serde_json::to_string(&Hero::Doctrine).unwrap(),
            "\"doctrine\""
        );
        let sombra: Hero = serde_json::from_str("\"sombra\"").unwrap();
        assert_eq!(sombra, Hero::Sombra);
        assert_eq!(serde_json::to_string(&Hero::Sombra).unwrap(), "\"sombra\"");
    }
}

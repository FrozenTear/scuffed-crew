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
    WatchpointGrimsvotn,

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
        WatchpointGrimsvotn,
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
            | Self::WatchpointGibraltar
            | Self::WatchpointGrimsvotn => GameMode::Escort,

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
            Self::WatchpointGrimsvotn => "Watchpoint: Grímsvötn",
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

/// Fold a live/OCR map string to a comparable key: lowercase, drop ASCII apostrophes,
/// treat `_`/`-`/`:` as spaces, and drop accents so `"Paraíso"` / `"Paraiso"`,
/// `"Esperança"` / `"Esperanca"`, and `"Watchpoint: Grímsvötn"` /
/// `"Watchpoint: Grimsvotn"` (also `Grímsvotn` / `Grimsvötn`) collide.
/// `ö`/`Ö` fold to `o` the same way `í` folds to `i`. Decomposed (NFD)
/// combining marks `\u{0300}`..=`\u{036f}` are dropped so the base letter remains,
/// including a mark produced when a character lowercases to a base letter plus a mark.
fn fold_map_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let mapped = match c {
            'Á' | 'À' | 'Ã' | 'Â' | 'á' | 'à' | 'ã' | 'â' => 'a',
            'É' | 'Ê' | 'é' | 'ê' => 'e',
            'Í' | 'í' => 'i',
            'Ó' | 'Ô' | 'Õ' | 'Ö' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'Ú' | 'Ü' | 'ú' | 'ü' => 'u',
            'Ç' | 'ç' => 'c',
            'Ñ' | 'ñ' => 'n',
            '\'' => continue,
            '_' | '-' | ':' => ' ',
            other => other,
        };
        // Drop combining marks here: NFD marks pass through `other` unchanged,
        // and `\u{0130}` lowercases to `i` + `\u{0307}`.
        for lower in mapped.to_lowercase() {
            if ('\u{0300}'..='\u{036f}').contains(&lower) {
                continue;
            }
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
            // Bare "watchpoint" stays Gibraltar so legacy rows do not move.
            // Bare "gibraltar" matches the tracker alias for the same map.
            "watchpoint gibraltar" | "watchpointgibraltar" | "watchpoint" | "gibraltar" => {
                Ok(Self::WatchpointGibraltar)
            }
            // Distinctive word, with or without the Watchpoint prefix, and the
            // no-space form. Diacritics fold to this key, precomposed or NFD
            // (grímsvötn, grimsvötn, grímsvotn, grimsvotn).
            "watchpoint grimsvotn" | "watchpointgrimsvotn" | "grimsvotn" => {
                Ok(Self::WatchpointGrimsvotn)
            }
            "blizzard world" | "blizzardworld" => Ok(Self::BlizzardWorld),
            // Adlersbrunn is the Halloween event label of Eichenwalde.
            // The alias matters for tracker 0.4.22+.
            "eichenwalde" | "adlersbrunn" => Ok(Self::Eichenwalde),
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
    fn all_covers_every_map_and_display_names_round_trip() {
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
    /// The first six strings are the ones that used to land in Other.
    /// Grímsvötn rows are the canonical name and the ASCII/diacritic folds.
    #[test]
    fn map_string_forms_classify_mode() {
        let cases = [
            ("Neon Junction", "Hybrid", MapName::NeonJunction),
            ("Paraíso", "Hybrid", MapName::Paraiso),
            ("Paraiso", "Hybrid", MapName::Paraiso),
            ("Esperança", "Push", MapName::Esperanca),
            ("Esperanca", "Push", MapName::Esperanca),
            ("neon junction", "Hybrid", MapName::NeonJunction),
            ("Parai\u{0301}so", "Hybrid", MapName::Paraiso),
            ("Esperanc\u{0327}a", "Push", MapName::Esperanca),
            (
                "Watchpoint: Grímsvötn",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            (
                "Watchpoint: Grimsvotn",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            (
                "Watchpoint: Grímsvotn",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            (
                "Watchpoint: Grimsvötn",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            (
                "Watchpoint: Gri\u{0301}msvo\u{0308}tn",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            (
                "WATCHPOINT: GRÍMSVÖTN",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            ("grimsvotn", "Escort", MapName::WatchpointGrimsvotn),
            ("grímsvötn", "Escort", MapName::WatchpointGrimsvotn),
            ("grimsvötn", "Escort", MapName::WatchpointGrimsvotn),
            ("grímsvotn", "Escort", MapName::WatchpointGrimsvotn),
            (
                "watchpoint grimsvotn",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            (
                "watchpointgrimsvotn",
                "Escort",
                MapName::WatchpointGrimsvotn,
            ),
            ("gibraltar", "Escort", MapName::WatchpointGibraltar),
            ("Gibraltar", "Escort", MapName::WatchpointGibraltar),
            ("Watchpoint", "Escort", MapName::WatchpointGibraltar),
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
                    "Escort" => GameMode::Escort,
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

    /// Gibraltar strings stay Gibraltar. Grímsvötn strings stay Grímsvötn.
    /// A bare `Watchpoint` is the Gibraltar alias and must not become Grímsvötn.
    /// Bare `gibraltar` is the tracker alias for Gibraltar. Bare `grimsvotn`
    /// (any diacritic fold) is Grímsvötn.
    #[test]
    fn watchpoint_grimsvotn_does_not_collide_with_gibraltar() {
        assert_eq!(
            MapName::WatchpointGrimsvotn.display_name(),
            "Watchpoint: Grímsvötn"
        );
        // Legacy rows that stored a bare "watchpoint" stay Gibraltar.
        assert_eq!(
            "watchpoint".parse::<MapName>(),
            Ok(MapName::WatchpointGibraltar)
        );
        assert_ne!(
            "watchpoint".parse::<MapName>(),
            Ok(MapName::WatchpointGrimsvotn)
        );
        // The prefixed phrase is Grímsvötn, not Gibraltar.
        assert_eq!(
            "watchpoint grimsvotn".parse::<MapName>(),
            Ok(MapName::WatchpointGrimsvotn)
        );
        assert_ne!(
            "watchpoint grimsvotn".parse::<MapName>(),
            Ok(MapName::WatchpointGibraltar)
        );
        let grimsvotn = [
            "Watchpoint: Grímsvötn",
            "Watchpoint: Grimsvotn",
            "Watchpoint: Grímsvotn",
            "Watchpoint: Grimsvötn",
            "Watchpoint: Gri\u{0301}msvo\u{0308}tn",
            "GR\u{0130}MSV\u{00d6}TN",
            "watchpoint: grímsvötn",
            "watchpoint grimsvotn",
            "watchpointgrimsvotn",
            "watchpointgrímsvötn",
            "watchpoint_grimsvotn",
            "grimsvotn",
            "grímsvötn",
            "grimsvötn",
            "grímsvotn",
            "gri\u{0301}msvo\u{0308}tn",
            "GRÍMSVÖTN",
            "Grimsvotn",
        ];
        let gibraltar = [
            "Watchpoint: Gibraltar",
            "watchpoint gibraltar",
            "watchpointgibraltar",
            "Watchpoint",
            "watchpoint",
            "WATCHPOINT",
            "watchpoint_gibraltar",
            "Watchpoint: gibraltar",
            "gibraltar",
            "Gibraltar",
            "GIBRALTAR",
        ];
        for name in grimsvotn {
            let parsed = MapName::from_str(name);
            assert_eq!(parsed, Ok(MapName::WatchpointGrimsvotn), "{name:?}");
            assert_ne!(parsed, Ok(MapName::WatchpointGibraltar), "{name:?}");
        }
        for name in gibraltar {
            let parsed = MapName::from_str(name);
            assert_eq!(parsed, Ok(MapName::WatchpointGibraltar), "{name:?}");
            assert_ne!(parsed, Ok(MapName::WatchpointGrimsvotn), "{name:?}");
        }
    }

    /// Inclusive ends of the combining-mark block parse; the code points
    /// just outside it do not.
    #[test]
    fn combining_mark_skip_includes_u0300_through_u036f_only() {
        assert_eq!(
            "grimsvotn\u{0300}".parse::<MapName>(),
            Ok(MapName::WatchpointGrimsvotn)
        );
        assert_eq!(
            "grimsvotn\u{036f}".parse::<MapName>(),
            Ok(MapName::WatchpointGrimsvotn)
        );
        assert_eq!("grimsvotn\u{02ff}".parse::<MapName>(), Err(()));
        assert_eq!("grimsvotn\u{0370}".parse::<MapName>(), Err(()));
    }

    /// Adlersbrunn is the Halloween event label of Eichenwalde.
    /// The alias matters for tracker 0.4.22+.
    #[test]
    fn adlersbrunn_halloween_label_folds_into_eichenwalde() {
        assert_eq!("Adlersbrunn".parse::<MapName>(), Ok(MapName::Eichenwalde));
        assert_eq!(MapName::game_mode_label("Adlersbrunn"), "Hybrid");
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

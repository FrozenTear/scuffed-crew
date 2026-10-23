//! `[auto_detect]`: the end-screen poller's settings, and the legacy
//! `poll_interval_secs = 4` that is read and written as automatic.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct AutoDetectConfig {
    pub enabled: bool,
    /// Seconds between end-screen polls. Missing means automatic
    /// ([`AutoDetectConfig::poll_interval`]): 1.5 s with the new reader,
    /// 4 s with classic OCR. 4, the value older Settings always wrote, also
    /// counts as automatic so existing configs get the faster rate.
    ///
    /// Automatic is written as 4, not left out: alpha.7 and older need the
    /// key, and read 4 as their own default, so a downgrade keeps working.
    /// Read back, 4 is `None` again, so automatic has one value in memory.
    #[serde(
        default,
        serialize_with = "write_auto_poll_as_legacy",
        deserialize_with = "read_legacy_poll_as_auto"
    )]
    pub poll_interval_secs: Option<u64>,
    /// Milliseconds between end-screen polls. When set, this wins over
    /// `poll_interval_secs` and over the automatic 1.5 s / 4 s interval.
    /// Clamped to 500..=10000. Missing leaves the seconds key in charge.
    /// Not written unless it was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u64>,
    pub cooldown_secs: u64,
}

impl Default for AutoDetectConfig {
    fn default() -> Self {
        Self {
            // The poller watches for the map-vote (game start) and post-match
            // accolade (win/loss) screens, required for automatic win/loss
            // detection, at ~25-35ms of CPU per tick. Enabled by default.
            enabled: true,
            poll_interval_secs: None,
            poll_interval_ms: None,
            // Debounce between opening new games: long enough that a map-vote
            // screen lingering across several ticks only opens one game, far
            // shorter than a real match so back-to-back games are still caught.
            cooldown_secs: 120,
        }
    }
}

/// Automatic poll interval with classic OCR (Tesseract reads the boards).
pub const CLASSIC_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(4);
/// Automatic poll interval with the new reader. Its end-screen checks are
/// cheap, and a 4 s tick missed short Victory or Defeat screens.
pub const NEW_READER_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);
/// The value older Settings wrote on every save; treated as automatic.
pub const LEGACY_DEFAULT_POLL_SECS: u64 = 4;
/// Lower and upper bounds for [`AutoDetectConfig::poll_interval_ms`].
pub const POLL_INTERVAL_MS_MIN: u64 = 500;
pub const POLL_INTERVAL_MS_MAX: u64 = 10_000;

fn read_legacy_poll_as_auto<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<u64>, D::Error> {
    let v = Option::<u64>::deserialize(de)?;
    Ok(v.filter(|&s| s != LEGACY_DEFAULT_POLL_SECS))
}

fn write_auto_poll_as_legacy<S: serde::Serializer>(
    v: &Option<u64>,
    ser: S,
) -> Result<S::Ok, S::Error> {
    ser.serialize_u64(v.unwrap_or(LEGACY_DEFAULT_POLL_SECS))
}

impl AutoDetectConfig {
    /// The poll interval in effect. `poll_interval_ms`, when set, wins over
    /// `poll_interval_secs` and over the automatic rate. Otherwise an
    /// explicit `poll_interval_secs` (at least 1 s) other than
    /// [`LEGACY_DEFAULT_POLL_SECS`] wins, else the automatic rate for the
    /// reader in use.
    pub fn poll_interval(&self, new_reader: bool) -> std::time::Duration {
        if let Some(ms) = self.poll_interval_ms {
            return std::time::Duration::from_millis(
                ms.clamp(POLL_INTERVAL_MS_MIN, POLL_INTERVAL_MS_MAX),
            );
        }
        match self.poll_interval_secs {
            Some(secs) if secs != LEGACY_DEFAULT_POLL_SECS => {
                std::time::Duration::from_secs(secs.max(1))
            }
            _ if new_reader => NEW_READER_POLL_INTERVAL,
            _ => CLASSIC_POLL_INTERVAL,
        }
    }

    /// Where [`Self::poll_interval`] came from, for the startup log.
    pub fn poll_interval_source(&self) -> &'static str {
        if self.poll_interval_ms.is_some() {
            "poll_interval_ms"
        } else if matches!(
            self.poll_interval_secs,
            Some(secs) if secs != LEGACY_DEFAULT_POLL_SECS
        ) {
            "poll_interval_secs"
        } else {
            "automatic"
        }
    }
}

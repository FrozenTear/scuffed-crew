//! Reading settings: `reader`, `ocr`, the shadow recognizer, the OCR
//! worker count and `debug_ocr`, with their env overrides.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::Config;

/// Env override for [`Config::ocr`]: `on` or `off`. Read by
/// [`Config::ocr_control`] only, never folded into the struct, so a Settings
/// save cannot write it to config.toml.
pub const OCR_ENV: &str = "STAT_TRACKER_OCR";

/// `ocr` in config.toml. Missing means [`Self::On`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrSetting {
    #[default]
    On,
    Off,
}

impl OcrSetting {
    pub const ON: &'static str = "on";
    pub const OFF: &'static str = "off";

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => Self::ON,
            Self::Off => Self::OFF,
        }
    }

    pub const fn is_on(&self) -> bool {
        matches!(self, Self::On)
    }

    pub const fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }

    /// `on` or `off`, in any case, with spaces around it ignored. The file
    /// value and the env override read the same way.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            Self::ON => Some(Self::On),
            Self::OFF => Some(Self::Off),
            _ => None,
        }
    }
}

impl Serialize for OcrSetting {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for OcrSetting {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| D::Error::custom("ocr must be \"on\" or \"off\""))
    }
}

/// Settings and daemon view of `ocr`.
///
/// `file` is the config.toml value, the only one Settings may write. `env`
/// is [`OCR_ENV`] for this process, when it is set to `on` or `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrControl {
    pub file: OcrSetting,
    pub env: Option<OcrSetting>,
}

impl OcrControl {
    /// What this process asks for: the env value when set, else the file.
    /// The daemon may still keep OCR on (`stat_tracker::ocr_switch::decide`).
    pub fn requested(self) -> OcrSetting {
        self.env.unwrap_or(self.file)
    }
}

/// `reader` in config.toml. Missing means [`Self::New`] (alpha.12 on;
/// before that, missing meant [`Self::OcrV1`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReaderSetting {
    OcrV1,
    #[default]
    New,
}

impl ReaderSetting {
    pub const OCR_V1: &'static str = "ocr-v1";
    pub const NEW: &'static str = "new";

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OcrV1 => Self::OCR_V1,
            Self::New => Self::NEW,
        }
    }

    pub const fn is_ocr_v1(&self) -> bool {
        matches!(self, Self::OcrV1)
    }

    /// The value a missing key means. A full save leaves it out and writes
    /// `"ocr-v1"`, so an explicit switch back survives a reload.
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::New)
    }

    pub const fn is_new(self) -> bool {
        matches!(self, Self::New)
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            Self::OCR_V1 => Some(Self::OcrV1),
            Self::NEW => Some(Self::New),
            _ => None,
        }
    }
}

impl Serialize for ReaderSetting {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReaderSetting {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| D::Error::custom("reader must be \"ocr-v1\" or \"new\""))
    }
}

/// Settings view of `shadow_recognizer`.
///
/// `file_on` is the config.toml value. `locked` means
/// `SCUFFED_SHADOW_RECOGNIZER` forces the extra reader on for this process.
/// A save writes `file_on` only. The checkbox can later become a reader
/// picker without a second env rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowRecognizerControl {
    pub file_on: bool,
    pub locked: bool,
}

impl ShadowRecognizerControl {
    /// On for this process. Same rule the daemon uses.
    pub fn enabled(self) -> bool {
        self.file_on || self.locked
    }
}

impl Config {
    pub fn debug_ocr_enabled(&self) -> bool {
        self.debug_ocr || Self::env_truthy("STAT_TRACKER_DEBUG_OCR")
    }

    /// Whether the shadow digit recognizer runs this process (config file
    /// and/or env `SCUFFED_SHADOW_RECOGNIZER`).
    ///
    /// The env override is read only by [`Self::shadow_recognizer_control`],
    /// so `shadow_recognizer` always holds the file value and a save never
    /// writes the override.
    pub fn shadow_recognizer_enabled(&self) -> bool {
        self.shadow_recognizer_control().enabled()
    }

    /// `reader = "new"`. Saved and uploaded stats may come from the new reader.
    pub fn uses_new_reader(&self) -> bool {
        self.reader.is_new()
    }

    /// File value of `ocr` plus this process's [`OCR_ENV`] override.
    ///
    /// The tracker service has its own environment, so Settings cannot treat
    /// `env` as the service's. `file` is what Settings may write.
    pub fn ocr_control(&self) -> OcrControl {
        Self::ocr_control_from(self.ocr, std::env::var(OCR_ENV).ok().as_deref())
    }

    /// Pure form of [`Self::ocr_control`]. `env` is the raw variable value.
    /// Only `on` and `off` count ([`OcrSetting::parse`]); anything else is
    /// no override.
    pub fn ocr_control_from(file: OcrSetting, env: Option<&str>) -> OcrControl {
        OcrControl {
            file,
            env: env.and_then(OcrSetting::parse),
        }
    }

    /// The shadow log runs when its own flag is on, or when saved stats use
    /// the new reader. The file flag stays independent of `reader`.
    pub fn shadow_log_enabled(&self) -> bool {
        self.shadow_recognizer_enabled() || self.uses_new_reader()
    }

    /// File value plus whether `SCUFFED_SHADOW_RECOGNIZER` locks this process on.
    ///
    /// This is the only read of that variable, and it is this process's
    /// environment. The tracker service is started by systemd with
    /// `session.env`, and it does not report the shadow reader back here, so
    /// Settings cannot treat this lock as the service's. `file_on` is what
    /// Settings may write. `locked` is display-only and must not be saved.
    pub fn shadow_recognizer_control(&self) -> ShadowRecognizerControl {
        Self::shadow_control(
            self.shadow_recognizer,
            std::env::var("SCUFFED_SHADOW_RECOGNIZER").ok().as_deref(),
        )
    }

    /// Pure form of [`Self::shadow_recognizer_control`] for tests and Settings.
    /// `env` is the raw variable value, not a process lookup.
    pub fn shadow_control(file_flag: bool, env: Option<&str>) -> ShadowRecognizerControl {
        ShadowRecognizerControl {
            file_on: file_flag,
            locked: Self::truthy(env),
        }
    }

    /// Resolved OCR worker count for the Rayon pool (and thus Tesseract instances).
    /// Explicit config/env/CLI wins; otherwise auto from host parallelism.
    pub fn ocr_threads_resolved(&self) -> usize {
        if let Some(n) = self.ocr_threads {
            return (n as usize).clamp(1, 8);
        }
        Self::default_ocr_threads()
    }

    /// Auto worker count when `ocr_threads` is unset: half the cores, 2..=4.
    pub fn default_ocr_threads() -> usize {
        let total = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        (total / 2).clamp(2, 4)
    }
}

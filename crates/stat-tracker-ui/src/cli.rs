use std::path::PathBuf;

use stat_tracker::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureKind {
    Empty,
    Sample,
    /// The Games tab's open-row states (UX Spec v2): a corrected game, an
    /// unsure one with a swap, and an undecided one, with Wrecking Ball
    /// history for the averages.
    Games,
}

impl FixtureKind {
    /// The `--fixture` value.
    pub fn name(self) -> &'static str {
        match self {
            FixtureKind::Empty => "empty",
            FixtureKind::Sample => "sample",
            FixtureKind::Games => "games",
        }
    }

    /// A fixture with games and the sample season list.
    pub fn has_games(self) -> bool {
        self != FixtureKind::Empty
    }
}

#[derive(Debug, Clone)]
pub struct Cli {
    pub data_dir: PathBuf,
    pub fixture: Option<FixtureKind>,
    pub seasons_url: Option<String>,
    pub companion: bool,
    pub help: bool,
    pub version: bool,
    /// Replace a running GUI ("Restart now" after an update): ask it to
    /// quit and wait for the single-instance lock.
    pub replace: bool,
}

/// `--build-id`: print [`crate::restart_prompt::BUILD_ID`] and exit. `main`
/// checks this before anything else (no config read, no instance lock, no
/// window), because a running GUI runs the installed binary with it to see
/// whether an update changed the app. It wins over every other flag.
pub fn wants_build_id(args: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    args.into_iter().any(|arg| arg.as_ref() == "--build-id")
}

/// The dir a `--fixture` run uses when no `--data-dir` is given: one per
/// kind under the temp dir. Only this dir starts each run with no queued
/// commands; a `--data-dir` the user passed keeps them.
pub fn default_fixture_dir(kind: FixtureKind) -> PathBuf {
    std::env::temp_dir().join(format!("scuffed-stat-tracker-ui-fixture-{}", kind.name()))
}

impl Cli {
    pub fn parse() -> Self {
        Self::parse_args(std::env::args().skip(1))
    }

    pub fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut data_dir = None;
        let mut fixture = None;
        let mut seasons_url = None;
        let mut companion = false;
        let mut help = false;
        let mut version = false;
        let mut replace = false;
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string())
            .collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-h" | "--help" => help = true,
                "-V" | "--version" => version = true,
                "--data-dir" => {
                    if let Some(v) = args.get(i + 1) {
                        data_dir = Some(PathBuf::from(v));
                        i += 1;
                    }
                }
                "--fixture" => {
                    if let Some(v) = args.get(i + 1) {
                        fixture = match v.as_str() {
                            "empty" => Some(FixtureKind::Empty),
                            "sample" => Some(FixtureKind::Sample),
                            "games" => Some(FixtureKind::Games),
                            other => {
                                eprintln!("unknown --fixture {other:?} (empty|sample|games)");
                                None
                            }
                        };
                        i += 1;
                    }
                }
                "--companion" => companion = true,
                "--replace" => replace = true,
                // Handled in `main` before parsing; see `wants_build_id`.
                "--build-id" => {}
                "--seasons-url" => {
                    if let Some(v) = args.get(i + 1) {
                        seasons_url = Some(v.clone());
                        i += 1;
                    }
                }
                other => eprintln!("unknown argument: {other}"),
            }
            i += 1;
        }

        let config = Config::load().unwrap_or_default();
        let data_dir = data_dir.unwrap_or_else(|| {
            // One dir per kind, so the games kit pictures never show up
            // in a later sample run.
            if let Some(kind) = fixture {
                default_fixture_dir(kind)
            } else {
                config.data_dir.clone()
            }
        });

        // Fixture runs never resolve a server URL, production `[]` must not
        // replace the sample season list written into the fixture cache.
        if fixture.is_none() && seasons_url.is_none() {
            if let Some(sync) = &config.sync
                && !sync.server_url.is_empty()
            {
                seasons_url = Some(crate::seasons::seasons_url_from_server(&sync.server_url));
            } else if let Ok(server) = std::env::var("SCUFFED_SERVER")
                && !server.trim().is_empty()
            {
                seasons_url = Some(crate::seasons::seasons_url_from_server(&server));
            }
        } else if fixture.is_some() {
            seasons_url = None;
        }

        Self {
            data_dir,
            fixture,
            seasons_url,
            companion,
            help,
            version,
            replace,
        }
    }

    pub fn help_text() -> &'static str {
        "stat-tracker-gui, Scuffed Crew tracker (Iced 0.14)

USAGE:
  stat-tracker-gui [OPTIONS]
  stat-tracker-gui --fixture sample
  stat-tracker-gui --fixture empty
  stat-tracker-gui --companion --fixture sample

OPTIONS:
  --data-dir PATH       Daemon data dir (default: config / XDG, or a temp dir with --fixture)
  --fixture empty|sample|games
                        Install a demo live_snapshot.json and read it back via storage::read_snapshot
  --companion           Run the layer-shell overlay only (no main window / tray).
                        Needs a Wayland compositor with wlr-layer-shell (niri).
  --seasons-url URL     GET /api/public/seasons on launch and every 30 min
                        (default: $SCUFFED_SERVER or config sync URL).
                        Cached to <data-dir>/seasons.json. Ignored when --fixture is set.
  --replace             Ask a running GUI to quit and take its place
                        (used by Restart now after an update)
  -h, --help            Show this help
  -V, --version         Print the tracker version and exit
  --build-id            Print this app's build id and exit (an update that
                        changes it asks a running window to restart)

ENVIRONMENT:
  SCUFFED_TRACKER_LAYOUT=classic
                        Start the 0.5 layout (sidebar and the old Settings
                        page with Save). Unset or tabs: the 0.6 top tabs.
"
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;

    #[test]
    fn version_flag_is_recognized() {
        let cli = Cli::parse_args(["--version"]);
        assert!(cli.version);
        assert!(!cli.help);
        let short = Cli::parse_args(["-V"]);
        assert!(short.version);
        let help = Cli::help_text();
        assert!(help.contains("--version"));
        assert!(help.contains("stat-tracker-gui [OPTIONS]"));
        assert!(!help.contains("cargo run"));
        assert!(!help.contains('\u{2014}'));
        assert!(!help.contains('\u{2013}'));
    }

    #[test]
    fn build_id_flag_wins_over_the_rest() {
        assert!(super::wants_build_id(["--build-id"]));
        assert!(super::wants_build_id(crate::restart_prompt::PROBE_ARGS));
        assert!(super::wants_build_id(["--replace", "--build-id"]));
        assert!(!super::wants_build_id(["--version", "--help"]));
        assert!(Cli::help_text().contains("--build-id"));
    }

    #[test]
    fn help_names_the_layout_switch() {
        let help = Cli::help_text();
        assert!(help.contains("ENVIRONMENT:"));
        assert!(help.contains(&format!("{}=classic", crate::nav::LAYOUT_ENV)));
    }
}

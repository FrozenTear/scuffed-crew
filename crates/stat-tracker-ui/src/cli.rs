use std::path::PathBuf;

use stat_tracker::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureKind {
    Empty,
    Sample,
}

#[derive(Debug, Clone)]
pub struct Cli {
    pub data_dir: PathBuf,
    pub fixture: Option<FixtureKind>,
    pub seasons_url: Option<String>,
    pub help: bool,
}

impl Cli {
    pub fn parse() -> Self {
        let mut data_dir = None;
        let mut fixture = None;
        let mut seasons_url = None;
        let mut help = false;
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-h" | "--help" => help = true,
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
                            other => {
                                eprintln!("unknown --fixture {other:?} (empty|sample)");
                                None
                            }
                        };
                        i += 1;
                    }
                }
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
            if fixture.is_some() {
                std::env::temp_dir().join("scuffed-stat-tracker-ui-fixture")
            } else {
                config.data_dir.clone()
            }
        });

        // Fixture runs never resolve a server URL — production `[]` must not
        // replace the sample season list written into the fixture cache.
        if fixture.is_none() && seasons_url.is_none() {
            if let Some(sync) = &config.sync
                && !sync.server_url.is_empty()
            {
                seasons_url = Some(crate::seasons::seasons_url_from_server(&sync.server_url));
            } else if let Ok(server) = std::env::var("SCUFFED_SERVER")
                && !server.is_empty()
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
            help,
        }
    }

    pub fn help_text() -> &'static str {
        "stat-tracker-gui — Scuffed Crew tracker (Iced 0.14, P1)

USAGE:
  cargo run -p scuffed-stat-tracker-ui -- [OPTIONS]
  cargo run -p scuffed-stat-tracker-ui -- --fixture sample
  cargo run -p scuffed-stat-tracker-ui -- --fixture empty

OPTIONS:
  --data-dir PATH       Daemon data dir (default: config / XDG, or a temp dir with --fixture)
  --fixture empty|sample
                        Install a demo live_snapshot.json and read it back via storage::read_snapshot
  --seasons-url URL     GET /api/public/seasons (default: $SCUFFED_SERVER or config sync URL).
                        Ignored when --fixture is set.
  -h, --help            Show this help
"
    }
}

use iced::Font;
use stat_tracker_ui::app::TrackerApp;
use stat_tracker_ui::cli::Cli;
use stat_tracker_ui::instance;
use stat_tracker_ui::theme;

fn main() -> anyhow::Result<()> {
    // Before logging, config, the instance lock and any window: a running
    // GUI runs the installed binary with this to compare builds (G3).
    if stat_tracker_ui::cli::wants_build_id(std::env::args().skip(1)) {
        println!("{}", stat_tracker_ui::restart_prompt::BUILD_ID);
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if cli.version {
        println!("{}", stat_tracker_ui::update::gui_version_line());
        return Ok(());
    }
    if cli.help {
        print!("{}", Cli::help_text());
        return Ok(());
    }

    // One GUI per session (#234). The companion overlay is a child process
    // of the GUI, and fixture runs are dev screenshots next to the real GUI,
    // so neither takes the lock. Keep the lock file open until exit.
    let _instance_lock = if cli.companion || cli.fixture.is_some() {
        None
    } else {
        let lock = instance::lock_path(std::env::var_os("XDG_RUNTIME_DIR"), &cli.data_dir);
        let version = stat_tracker_ui::update::daemon_package_version();
        let claimed = instance::claim_session(
            &lock,
            &cli.data_dir,
            &version,
            cli.replace,
            std::path::Path::new("/proc"),
            instance::REPLACE_WAIT,
        );
        match claimed {
            Ok(instance::Claim::Primary(file)) => {
                // A GUI from before the lock (0.4.x), or one left on a
                // replaced binary without it, keeps running beside this
                // one, and its Save writes an older config over this one's
                // settings (S5-UP-1). Stop it, unless it is mid-update.
                match instance::Me::current() {
                    Some(me) => {
                        let stopped = instance::stop_stale_guis(
                            instance::ScanRoots::live(),
                            me,
                            &mut instance::sigterm_if_exe,
                        );
                        if !stopped.is_empty() {
                            tracing::info!(
                                ?stopped,
                                "stopped older tracker GUIs that ran without the lock"
                            );
                        }
                    }
                    None => tracing::warn!(
                        "could not read this process's start time; older GUIs are left running"
                    ),
                }
                Some(file)
            }
            Ok(instance::Claim::Secondary) => {
                tracing::info!(lock = %lock.display(), "tracker GUI already running; asking it to show");
                if let Err(e) = instance::request_show(&cli.data_dir) {
                    tracing::warn!(error = %e, "could not ask the running GUI to show");
                }
                return Ok(());
            }
            Err(e) => {
                // Never block a launch on the lock itself.
                tracing::warn!(error = %e, lock = %lock.display(), "single-instance lock unavailable; starting anyway");
                None
            }
        }
    };

    // tray-icon on Linux needs GTK before the menu is built. Failure is fine
    // the Iced window still runs. AppIndicator is probed in `tray::try_create`;
    // a missing libayatana-appindicator3 must not panic (AerynOS).
    if gtk::init().is_err() {
        tracing::info!("system tray unavailable (no display / GTK)");
    }

    let data_dir = cli.data_dir.clone();
    if cli.companion {
        tracing::info!(path = %data_dir.display(), "starting companion overlay");
        return stat_tracker_ui::overlay::run_companion(cli);
    }

    tracing::info!(path = %data_dir.display(), "starting tracker GUI (Iced 0.14 P4 + P3 overlay)");

    // Daemon (not application): the process stays alive with zero windows so
    // tray Hide can `window::close` the surface. Mode::Hidden / set_visible
    // leave the window in niri's Alt-Tab list. The companion overlay is a
    // second process (`--companion` / iced_layershell), not a second iced
    // window on this runtime.
    iced::daemon(
        {
            let cli = cli.clone();
            move || TrackerApp::new(cli.clone())
        },
        TrackerApp::update,
        TrackerApp::view,
    )
    .title(TrackerApp::title)
    .theme(theme::iced_theme())
    .subscription(TrackerApp::subscription)
    .font(theme::FONT_BYTES_MEDIUM)
    .font(theme::FONT_BYTES_SEMIBOLD)
    .font(theme::FONT_BYTES_BOLD)
    .font(theme::FONT_BYTES_EXTRABOLD)
    .default_font(Font {
        family: iced::font::Family::Name("Urbanist"),
        weight: iced::font::Weight::Medium,
        stretch: iced::font::Stretch::Normal,
        style: iced::font::Style::Normal,
    })
    .run()?;
    Ok(())
}

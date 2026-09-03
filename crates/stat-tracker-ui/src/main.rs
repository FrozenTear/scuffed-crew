use iced::Font;
use stat_tracker_ui::app::TrackerApp;
use stat_tracker_ui::cli::Cli;
use stat_tracker_ui::theme;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if cli.help {
        print!("{}", Cli::help_text());
        return Ok(());
    }

    // tray-icon on Linux needs GTK before the menu is built. Failure is fine
    // — the Iced window still runs. AppIndicator is probed in `tray::try_create`;
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

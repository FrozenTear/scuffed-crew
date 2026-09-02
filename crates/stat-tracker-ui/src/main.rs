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
    // on headless hosts — the window still runs without a tray.
    if gtk::init().is_err() {
        tracing::info!("system tray unavailable (no display / GTK)");
    }

    let data_dir = cli.data_dir.clone();
    tracing::info!(path = %data_dir.display(), "starting tracker GUI (Iced 0.14 P4)");

    // Daemon (not application): the process stays alive with zero windows so
    // tray Hide can `window::close` the surface. Mode::Hidden / set_visible
    // leave the window in niri's Alt-Tab list.
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

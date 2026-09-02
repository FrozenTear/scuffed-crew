use iced::Font;
use iced::window;
use stat_tracker_ui::app::TrackerApp;
use stat_tracker_ui::cli::Cli;
use stat_tracker_ui::preview;
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

    if let Some(path) = cli.preview.clone() {
        let (app, _task) = TrackerApp::new(cli);
        preview::write_overview_png(&app, &path)?;
        println!("wrote {}", path.display());
        return Ok(());
    }

    let data_dir = cli.data_dir.clone();
    tracing::info!(path = %data_dir.display(), "starting Overview (Iced 0.14)");

    iced::application(
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
    .window(window::Settings {
        size: iced::Size::new(1280.0, 860.0),
        min_size: Some(iced::Size::new(960.0, 640.0)),
        ..window::Settings::default()
    })
    .run()?;
    Ok(())
}

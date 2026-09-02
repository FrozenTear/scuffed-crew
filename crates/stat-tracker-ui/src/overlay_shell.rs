//! iced_layershell runner for `--companion`. Feature-gated so a host that
//! cannot build iced_layershell can still compile visibility tests.

use std::time::Duration;

use iced::{Color, Element, Font, Task};
use iced_layershell::application;
use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;

use crate::cli::Cli;
use crate::overlay::{
    OVERLAY_NAMESPACE, OverlayApp, content_height, overlay_shell_spec, view as overlay_view,
};
use crate::theme;

#[to_layer_message]
#[derive(Debug, Clone)]
enum Message {
    Tick,
}

fn namespace() -> String {
    OVERLAY_NAMESPACE.into()
}

fn update(app: &mut OverlayApp, message: Message) -> Task<Message> {
    match message {
        Message::Tick => {
            app.refresh();
            Task::none()
        }
        _ => Task::none(),
    }
}

fn view(app: &OverlayApp) -> Element<'_, Message> {
    overlay_view(&app.model).map(|_| Message::Tick)
}

fn style(_app: &OverlayApp, theme: &iced::Theme) -> iced::theme::Style {
    iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: theme.palette().text,
    }
}

fn overlay_theme(_app: &OverlayApp) -> iced::Theme {
    theme::iced_theme()
}

fn overlay_subscription(_app: &OverlayApp) -> iced::Subscription<Message> {
    iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    iced_layershell::disable_clipboard();

    let boot_cli = cli.clone();
    let config = stat_tracker::config::Config::load().unwrap_or_default();
    let preview = OverlayApp::load(&cli);
    let spec = overlay_shell_spec(
        content_height(&preview.model),
        config.capture_output.as_deref(),
    );
    let start_mode = match spec.output.clone() {
        Some(name) => StartMode::TargetScreen(name),
        None => StartMode::Active,
    };

    tracing::info!(
        width = spec.width,
        height = spec.height,
        output = ?spec.output,
        "starting companion overlay (iced_layershell)"
    );

    application(move || OverlayApp::load(&boot_cli), namespace, update, view)
        .style(style)
        .subscription(overlay_subscription)
        .theme(overlay_theme)
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
        .settings(Settings {
            id: Some(OVERLAY_NAMESPACE.into()),
            layer_settings: LayerShellSettings {
                size: Some((spec.width, spec.height)),
                exclusive_zone: spec.exclusive_zone,
                anchor: Anchor::Top | Anchor::Right,
                margin: (spec.margin, spec.margin, spec.margin, spec.margin),
                layer: Layer::Overlay,
                keyboard_interactivity: KeyboardInteractivity::None,
                start_mode,
                events_transparent: spec.events_transparent,
            },
            ..Default::default()
        })
        .run()
        .map_err(|e| anyhow::anyhow!("companion overlay: {e}"))
}

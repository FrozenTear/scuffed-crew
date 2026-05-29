use std::io::Cursor;

use base64::Engine;
use dioxus::prelude::*;
use image::DynamicImage;

use stat_tracker::capture;
use stat_tracker::config::Config;

fn encode_thumbnail(img: &DynamicImage) -> String {
    let thumb = img.thumbnail(800, 450);
    let mut buf = Cursor::new(Vec::new());
    thumb
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .expect("jpeg encode");
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
    format!("data:image/jpeg;base64,{b64}")
}

#[component]
pub fn PreviewPanel() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut preview_src: Signal<Option<String>> = use_signal(|| None);
    let mut preview_time: Signal<Option<String>> = use_signal(|| None);
    let mut capturing = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    let capture_now = move |_| {
        let output = config().capture_output.clone();
        capturing.set(true);
        error_msg.set(None);
        spawn(async move {
            let backend = capture::detect_backend().await;
            match capture::capture_screen_output(&backend, output.as_deref()).await {
                Ok(img) => {
                    let data_uri =
                        tokio::task::spawn_blocking(move || encode_thumbnail(&img)).await;
                    match data_uri {
                        Ok(uri) => {
                            preview_src.set(Some(uri));
                            preview_time
                                .set(Some(chrono::Local::now().format("%H:%M:%S").to_string()));
                        }
                        Err(e) => error_msg.set(Some(format!("Encode failed: {e}"))),
                    }
                }
                Err(e) => error_msg.set(Some(format!("Capture failed: {e}"))),
            }
            capturing.set(false);
        });
    };

    let output_label = config()
        .capture_output
        .clone()
        .unwrap_or_else(|| "auto (first output)".into());

    rsx! {
        div { class: "panel",
            h2 { "Capture Preview" }

            div { class: "card",
                h3 { "Current output" }
                div { class: "stat-row",
                    span { class: "label", "Target" }
                    span { class: "value", "{output_label}" }
                }
                div { class: "actions",
                    button {
                        class: "btn btn-primary",
                        disabled: capturing(),
                        onclick: capture_now,
                        if capturing() { "Capturing..." } else { "Capture Now" }
                    }
                }
            }

            if let Some(ref err) = error_msg() {
                div { class: "card card-error",
                    p { "{err}" }
                }
            }

            if let Some(ref src) = preview_src() {
                div { class: "card",
                    h3 {
                        "Preview"
                        if let Some(ref t) = preview_time() {
                            span { class: "text-dim", " — {t}" }
                        }
                    }
                    img {
                        class: "preview-img",
                        src: "{src}",
                        alt: "Screen capture preview",
                    }
                    p { class: "text-dim text-sm",
                        "This is what the stat tracker sees. Make sure the Overwatch scoreboard is visible."
                    }
                }
            }
        }
    }
}

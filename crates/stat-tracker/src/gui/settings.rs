use std::path::PathBuf;

use dioxus::prelude::*;

use stat_tracker::config::{AutoDetectConfig, Config, SyncConfig};

#[component]
pub fn SettingsPanel() -> Element {
    let mut config = use_signal(|| Config::load().unwrap_or_default());
    let outputs = use_signal(|| {
        stat_tracker::capture::wayshot::list_outputs().unwrap_or_default()
    });
    let mut toast_msg: Signal<Option<String>> = use_signal(|| None);

    let mut player_name = use_signal(|| {
        config().player_name.clone().unwrap_or_default()
    });
    let mut capture_output = use_signal(|| {
        config().capture_output.clone().unwrap_or_default()
    });
    let mut auto_detect_enabled = use_signal(|| config().auto_detect.enabled);
    let mut poll_interval = use_signal(|| config().auto_detect.poll_interval_secs.to_string());
    let mut cooldown = use_signal(|| config().auto_detect.cooldown_secs.to_string());
    let mut sync_url = use_signal(|| {
        config().sync.as_ref().map(|s| s.server_url.clone()).unwrap_or_default()
    });
    let mut sync_token = use_signal(|| {
        config().sync.as_ref().map(|s| s.token.clone()).unwrap_or_default()
    });

    let save = move |_| {
        let new_config = Config {
            data_dir: config().data_dir.clone(),
            capture_output: if capture_output().is_empty() {
                None
            } else {
                Some(capture_output())
            },
            player_name: if player_name().is_empty() {
                None
            } else {
                Some(player_name())
            },
            sync: if sync_url().is_empty() || sync_token().is_empty() {
                None
            } else {
                Some(SyncConfig {
                    server_url: sync_url(),
                    token: sync_token(),
                })
            },
            auto_detect: AutoDetectConfig {
                enabled: auto_detect_enabled(),
                poll_interval_secs: poll_interval().parse().unwrap_or(4),
                cooldown_secs: cooldown().parse().unwrap_or(120),
            },
        };

        match save_config(&new_config) {
            Ok(()) => {
                config.set(new_config);
                toast_msg.set(Some("Settings saved!".into()));
                spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    toast_msg.set(None);
                });
            }
            Err(e) => {
                toast_msg.set(Some(format!("Save failed: {e}")));
            }
        }
    };

    let install_tessdata = move |_| {
        spawn(async move {
            match tokio::task::spawn_blocking(stat_tracker::setup::ensure_koverwatch_tessdata).await
            {
                Ok(Ok(())) => toast_msg.set(Some("Koverwatch tessdata installed!".into())),
                Ok(Err(e)) => toast_msg.set(Some(format!("Failed: {e}"))),
                Err(e) => toast_msg.set(Some(format!("Task error: {e}"))),
            }
        });
    };

    rsx! {
        div { class: "panel",
            h2 { "Settings" }

            div { class: "card",
                h3 { "Capture" }
                div { class: "field",
                    label { "Output (monitor)" }
                    select {
                        value: "{capture_output}",
                        onchange: move |e| capture_output.set(e.value()),
                        option { value: "", "Auto (first output)" }
                        for name in outputs().iter() {
                            option {
                                value: "{name}",
                                selected: capture_output() == *name,
                                "{name}"
                            }
                        }
                    }
                }
                div { class: "field",
                    label { "Player name (battletag)" }
                    input {
                        r#type: "text",
                        placeholder: "e.g. YourName#1234",
                        value: "{player_name}",
                        oninput: move |e| player_name.set(e.value()),
                    }
                }
            }

            div { class: "card",
                h3 { "Auto-detect" }
                div { class: "checkbox-row",
                    input {
                        r#type: "checkbox",
                        checked: auto_detect_enabled(),
                        onchange: move |e| auto_detect_enabled.set(e.checked()),
                    }
                    label { "Enable match-end detection polling" }
                }
                if auto_detect_enabled() {
                    div { class: "field",
                        label { "Poll interval (seconds)" }
                        input {
                            r#type: "number",
                            value: "{poll_interval}",
                            oninput: move |e| poll_interval.set(e.value()),
                        }
                    }
                    div { class: "field",
                        label { "Cooldown after detection (seconds)" }
                        input {
                            r#type: "number",
                            value: "{cooldown}",
                            oninput: move |e| cooldown.set(e.value()),
                        }
                    }
                }
            }

            div { class: "card",
                h3 { "Server sync" }
                div { class: "field",
                    label { "Server URL" }
                    input {
                        r#type: "url",
                        placeholder: "https://your-site.com",
                        value: "{sync_url}",
                        oninput: move |e| sync_url.set(e.value()),
                    }
                }
                div { class: "field",
                    label { "Daemon token" }
                    input {
                        r#type: "password",
                        placeholder: "paste token from web UI",
                        value: "{sync_token}",
                        oninput: move |e| sync_token.set(e.value()),
                    }
                }
            }

            div { class: "card",
                h3 { "OCR Setup" }
                button {
                    class: "btn btn-secondary",
                    onclick: install_tessdata,
                    "Install / Reinstall Koverwatch Tessdata"
                }
            }

            div { class: "actions",
                button { class: "btn btn-primary", onclick: save, "Save Settings" }
            }

            if let Some(msg) = toast_msg() {
                div { class: "toast success", "{msg}" }
            }
        }
    }
}

fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = dirs::config_dir()
        .ok_or("no config directory")?
        .join("scuffed-stat-tracker");
    std::fs::create_dir_all(&config_dir)?;

    let toml_str = serialize_config(config);
    std::fs::write(config_dir.join("config.toml"), toml_str)?;
    Ok(())
}

fn serialize_config(config: &Config) -> String {
    let mut lines = Vec::new();

    let default_data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("scuffed-stat-tracker");

    if config.data_dir != default_data_dir {
        lines.push(format!(
            "data_dir = {:?}",
            config.data_dir.to_str().unwrap_or("")
        ));
    }

    if let Some(ref output) = config.capture_output {
        lines.push(format!("capture_output = {output:?}"));
    }

    if let Some(ref name) = config.player_name {
        lines.push(format!("player_name = {name:?}"));
    }

    if !lines.is_empty() {
        lines.push(String::new());
    }

    lines.push("[auto_detect]".into());
    lines.push(format!("enabled = {}", config.auto_detect.enabled));
    lines.push(format!(
        "poll_interval_secs = {}",
        config.auto_detect.poll_interval_secs
    ));
    lines.push(format!("cooldown_secs = {}", config.auto_detect.cooldown_secs));

    if let Some(ref sync) = config.sync {
        lines.push(String::new());
        lines.push("[sync]".into());
        lines.push(format!("server_url = {:?}", sync.server_url));
        lines.push(format!("token = {:?}", sync.token));
    }

    lines.push(String::new());
    lines.join("\n")
}

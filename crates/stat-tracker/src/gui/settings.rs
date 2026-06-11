use dioxus::prelude::*;

use stat_tracker::config::{AutoDetectConfig, Config, SyncConfig};

#[component]
pub fn SettingsPanel() -> Element {
    let mut config = use_signal(|| Config::load().unwrap_or_default());
    let outputs = use_signal(|| stat_tracker::capture::wayshot::list_outputs().unwrap_or_default());
    let mut toast: Signal<Option<(String, bool)>> = use_signal(|| None);

    let mut player_name = use_signal(|| config().player_name.clone().unwrap_or_default());
    let mut capture_output = use_signal(|| config().capture_output.clone().unwrap_or_default());
    let mut auto_detect_enabled = use_signal(|| config().auto_detect.enabled);
    let mut poll_interval = use_signal(|| config().auto_detect.poll_interval_secs.to_string());
    let mut cooldown = use_signal(|| config().auto_detect.cooldown_secs.to_string());
    let mut sync_url = use_signal(|| {
        config()
            .sync
            .as_ref()
            .map(|s| s.server_url.clone())
            .unwrap_or_default()
    });
    let mut sync_token = use_signal(|| {
        config()
            .sync
            .as_ref()
            .map(|s| s.token.clone())
            .unwrap_or_default()
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
            session_window_secs: config().session_window_secs,
            game_process_names: config().game_process_names.clone(),
        };

        match save_config(&new_config) {
            Ok(()) => {
                config.set(new_config);
                toast.set(Some(("Settings saved!".into(), true)));
                spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    toast.set(None);
                });
            }
            Err(e) => {
                toast.set(Some((format!("Save failed: {e}"), false)));
                spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    toast.set(None);
                });
            }
        }
    };

    let install_tessdata = move |_| {
        spawn(async move {
            match tokio::task::spawn_blocking(stat_tracker::setup::ensure_koverwatch_tessdata).await
            {
                Ok(Ok(())) => toast.set(Some(("Koverwatch tessdata installed!".into(), true))),
                Ok(Err(e)) => toast.set(Some((format!("Failed: {e}"), false))),
                Err(e) => toast.set(Some((format!("Task error: {e}"), false))),
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

            div { class: "card card-warning",
                h3 { "Data Management" }
                p { class: "text-dim text-sm", "Clear all stored match data, sessions, and logs. This cannot be undone. If the daemon is running, restart it after clearing." }
                div { class: "actions",
                    button {
                        class: "btn btn-danger",
                        onclick: {
                            let data_dir = config().data_dir.clone();
                            move |_| {
                                let data_dir = data_dir.clone();
                                spawn(async move {
                                    let result = async {
                                        match stat_tracker::storage::LocalStore::open(&data_dir).await {
                                            Ok(store) => {
                                                store.clear_all_data().await?;
                                                stat_tracker::storage::clear_match_log(&data_dir);
                                            }
                                            Err(_) => {
                                                stat_tracker::storage::force_clear_data_dir(&data_dir)?;
                                            }
                                        }
                                        Ok::<(), Box<dyn std::error::Error>>(())
                                    }.await;
                                    match result {
                                        Ok(()) => {
                                            toast.set(Some(("All match data cleared!".into(), true)));
                                            spawn(async move {
                                                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                                                toast.set(None);
                                            });
                                        }
                                        Err(e) => {
                                            toast.set(Some((format!("Clear failed: {e}"), false)));
                                            spawn(async move {
                                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                                toast.set(None);
                                            });
                                        }
                                    }
                                });
                            }
                        },
                        "Clear All Match Data"
                    }
                }
            }

            div { class: "actions",
                button { class: "btn btn-primary", onclick: save, "Save Settings" }
            }

            if let Some((msg, ok)) = toast() {
                div { class: if ok { "toast success" } else { "toast error" }, "{msg}" }
            }
        }
    }
}

fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = dirs::config_dir()
        .ok_or("no config directory")?
        .join("scuffed-stat-tracker");
    std::fs::create_dir_all(&config_dir)?;

    let toml_str = toml::to_string_pretty(config)?;
    std::fs::write(config_dir.join("config.toml"), toml_str)?;
    Ok(())
}

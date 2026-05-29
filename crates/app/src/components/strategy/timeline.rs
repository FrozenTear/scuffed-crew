use dioxus::prelude::*;
use scuffed_types::{PlaybackState, TimelinePhase};
use uuid::Uuid;

const TIMELINE_CSS: &str = r#"
    .timeline-panel {
        display: flex;
        flex-direction: column;
        background: var(--bg-surface);
        border-top: 1px solid var(--border);
        overflow-y: auto;
    }
    .timeline-panel .panel-title {
        font-family: var(--font-display);
        font-size: 0.75rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        padding: 0.75rem 0.75rem 0.5rem;
        margin: 0;
        border-bottom: 1px solid var(--border);
    }

    /* ---- Phase list ---- */
    .timeline-phases {
        display: flex;
        flex-direction: column;
    }
    .timeline-phase {
        display: flex;
        align-items: stretch;
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--border);
        cursor: pointer;
        transition: background 0.12s;
    }
    .timeline-phase:hover {
        background: var(--bg-card);
    }
    .timeline-phase.active {
        background: var(--accent-soft);
    }

    /* ---- Phase marker (visual timeline dot + line) ---- */
    .phase-marker {
        display: flex;
        flex-direction: column;
        align-items: center;
        width: 20px;
        margin-right: 0.5rem;
    }
    .phase-dot {
        width: 10px;
        height: 10px;
        border-radius: 50%;
        background: var(--text-muted);
        flex-shrink: 0;
    }
    .timeline-phase.active .phase-dot {
        background: var(--accent);
        box-shadow: 0 0 6px var(--accent-glow);
    }
    .phase-line {
        width: 2px;
        flex: 1;
        background: var(--border);
        margin-top: 0.2rem;
    }

    /* ---- Phase content ---- */
    .phase-content {
        flex: 1;
        display: flex;
        align-items: center;
        justify-content: space-between;
    }
    .phase-header {
        display: flex;
        align-items: center;
        gap: 0.5rem;
    }
    .phase-name {
        font-size: 0.8rem;
        color: var(--text-secondary);
    }
    .timeline-phase.active .phase-name {
        color: var(--accent-bright);
        font-weight: 600;
    }
    .phase-timestamp {
        font-size: 0.65rem;
        color: var(--text-muted);
        background: var(--bg-card);
        padding: 0.1rem 0.3rem;
        border-radius: 3px;
    }
    .phase-actions {
        display: flex;
        gap: 0.2rem;
    }
    .phase-action-btn {
        background: none;
        border: none;
        color: var(--text-muted);
        font-size: 0.75rem;
        cursor: pointer;
        padding: 0.1rem 0.3rem;
        border-radius: 3px;
        transition: color 0.12s, background 0.12s;
    }
    .phase-action-btn:hover {
        color: #ef5350;
        background: rgba(239, 83, 80, 0.1);
    }

    /* ---- Add phase form ---- */
    .add-phase-form {
        display: flex;
        gap: 0.3rem;
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--border);
    }
    .add-phase-form input {
        flex: 1;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 4px;
        color: var(--text-bright);
        font-size: 0.8rem;
        padding: 0.3rem 0.4rem;
        outline: none;
        transition: border-color 0.15s;
    }
    .add-phase-form input:focus {
        border-color: var(--accent);
    }
    .add-phase-btn {
        padding: 0.3rem 0.6rem;
        border: none;
        border-radius: 4px;
        background: var(--accent);
        color: #fff;
        font-size: 0.85rem;
        font-weight: 700;
        cursor: pointer;
        transition: opacity 0.12s;
    }
    .add-phase-btn:hover {
        opacity: 0.9;
    }

    /* ---- Playback controls ---- */
    .timeline-controls {
        display: flex;
        justify-content: center;
        gap: 0.3rem;
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--border);
    }
    .tl-ctrl-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 28px;
        border: 1px solid var(--border);
        border-radius: 4px;
        background: none;
        color: var(--text-secondary);
        font-size: 0.75rem;
        cursor: pointer;
        transition: background 0.12s, color 0.12s;
    }
    .tl-ctrl-btn:hover {
        background: var(--bg-card);
        color: var(--text-bright);
    }
    .tl-ctrl-btn.playing {
        background: var(--accent-soft);
        color: var(--accent-bright);
        border-color: var(--accent);
    }

    /* ---- Progress indicator ---- */
    .timeline-progress {
        text-align: center;
        font-size: 0.7rem;
        color: var(--text-muted);
        padding: 0.35rem;
    }

    /* ---- Info hint ---- */
    .timeline-info {
        padding: 0.5rem 0.75rem;
    }
    .timeline-info .hint {
        font-size: 0.7rem;
        color: var(--text-muted);
        opacity: 0.7;
        margin: 0;
    }
"#;

#[component]
pub fn Timeline(
    /// All phases.
    phases: Vec<TimelinePhase>,

    /// Currently selected phase.
    #[props(default)]
    selected_phase: Option<Uuid>,

    /// Playback state.
    playback_state: PlaybackState,

    // ---- Callbacks ----
    on_select_phase: EventHandler<Option<Uuid>>,
    on_add_phase: EventHandler<String>,
    on_delete_phase: EventHandler<Uuid>,

    on_play_pause: EventHandler<()>,
    on_next_phase: EventHandler<()>,
    on_prev_phase: EventHandler<()>,
    on_first_phase: EventHandler<()>,
    on_last_phase: EventHandler<()>,
) -> Element {
    let mut new_phase_name = use_signal(String::new);

    let total = phases.len();
    let current_index = selected_phase
        .and_then(|sel| phases.iter().position(|p| p.id == sel).map(|i| i + 1))
        .unwrap_or(0);

    let is_playing = playback_state == PlaybackState::Playing;

    rsx! {
        style { {TIMELINE_CSS} }
        div { class: "timeline-panel",
            h3 { class: "panel-title", "Timeline" }

            // ---- Phase list ----
            div { class: "timeline-phases",
                {phases.iter().map(|phase| {
                    let phase_id = phase.id;
                    let phase_name = phase.name.clone();
                    let timestamp = phase.timestamp.clone();
                    let is_active = selected_phase == Some(phase_id);
                    let row_cls = if is_active { "timeline-phase active" } else { "timeline-phase" };

                    rsx! {
                        div {
                            class: "{row_cls}",
                            onclick: move |_| {
                                let new_sel = if selected_phase == Some(phase_id) {
                                    None
                                } else {
                                    Some(phase_id)
                                };
                                on_select_phase.call(new_sel);
                            },
                            div { class: "phase-marker",
                                div { class: "phase-dot" }
                                div { class: "phase-line" }
                            }
                            div { class: "phase-content",
                                div { class: "phase-header",
                                    span { class: "phase-name", "{phase_name}" }
                                    if let Some(ref ts) = timestamp {
                                        span { class: "phase-timestamp", "{ts}" }
                                    }
                                }
                                div { class: "phase-actions",
                                    button {
                                        class: "phase-action-btn",
                                        title: "Delete",
                                        onclick: move |e: Event<MouseData>| {
                                            e.stop_propagation();
                                            on_delete_phase.call(phase_id);
                                        },
                                        "x"
                                    }
                                }
                            }
                        }
                    }
                })}
            }

            // ---- Add phase ----
            div { class: "add-phase-form",
                input {
                    r#type: "text",
                    placeholder: "New phase name...",
                    value: "{new_phase_name}",
                    oninput: move |e: Event<FormData>| {
                        new_phase_name.set(e.value());
                    },
                    onkeypress: move |e: Event<KeyboardData>| {
                        if e.data().key() == Key::Enter {
                            let name = new_phase_name();
                            if !name.is_empty() {
                                on_add_phase.call(name);
                                new_phase_name.set(String::new());
                            }
                        }
                    },
                }
                button {
                    class: "add-phase-btn",
                    onclick: move |_| {
                        let name = new_phase_name();
                        if !name.is_empty() {
                            on_add_phase.call(name);
                            new_phase_name.set(String::new());
                        }
                    },
                    "+"
                }
            }

            // ---- Playback controls ----
            div { class: "timeline-controls",
                button {
                    class: "tl-ctrl-btn",
                    title: "First Phase",
                    onclick: move |_| on_first_phase.call(()),
                    "|<"
                }
                button {
                    class: "tl-ctrl-btn",
                    title: "Previous Phase",
                    onclick: move |_| on_prev_phase.call(()),
                    "<"
                }
                button {
                    class: if is_playing { "tl-ctrl-btn playing" } else { "tl-ctrl-btn" },
                    title: if is_playing { "Pause" } else { "Play" },
                    onclick: move |_| on_play_pause.call(()),
                    if is_playing { "||" } else { ">" }
                }
                button {
                    class: "tl-ctrl-btn",
                    title: "Next Phase",
                    onclick: move |_| on_next_phase.call(()),
                    ">"
                }
                button {
                    class: "tl-ctrl-btn",
                    title: "Last Phase",
                    onclick: move |_| on_last_phase.call(()),
                    ">|"
                }
            }

            // ---- Progress ----
            div { class: "timeline-progress",
                "Phase {current_index} / {total}"
            }

            // ---- Hint ----
            div { class: "timeline-info",
                p { class: "hint",
                    "Phases let you create step-by-step strategies. Elements can be assigned to specific phases."
                }
            }
        }
    }
}

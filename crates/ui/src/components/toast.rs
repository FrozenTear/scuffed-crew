use leptos::prelude::*;
use leptos::task::spawn_local;
use uuid::Uuid;

/// Toast notification severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl ToastLevel {
    pub fn css_class(&self) -> &'static str {
        match self {
            ToastLevel::Info => "sc-toast-info",
            ToastLevel::Success => "sc-toast-success",
            ToastLevel::Warning => "sc-toast-warning",
            ToastLevel::Error => "sc-toast-error",
        }
    }
}

/// A toast notification message
#[derive(Debug, Clone)]
pub struct Toast {
    pub id: Uuid,
    pub message: String,
    pub level: ToastLevel,
}

impl Toast {
    pub fn new(message: impl Into<String>, level: ToastLevel) -> Self {
        Self {
            id: Uuid::new_v4(),
            message: message.into(),
            level,
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Info)
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Success)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Warning)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Error)
    }
}

/// Toast notification state manager
#[derive(Clone, Copy)]
pub struct ToastState {
    pub toasts: RwSignal<Vec<Toast>>,
}

impl ToastState {
    pub fn new() -> Self {
        Self {
            toasts: RwSignal::new(Vec::new()),
        }
    }

    /// Show a toast notification (auto-dismisses after 3 seconds)
    pub fn show(&self, toast: Toast) {
        let toast_id = toast.id;
        self.toasts.update(|toasts| {
            toasts.push(toast);
        });

        let toasts_signal = self.toasts;
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(3000).await;
            toasts_signal.update(|toasts| {
                toasts.retain(|t| t.id != toast_id);
            });
        });
    }

    /// Dismiss a specific toast
    pub fn dismiss(&self, id: Uuid) {
        self.toasts.update(|toasts| {
            toasts.retain(|t| t.id != id);
        });
    }
}

impl Default for ToastState {
    fn default() -> Self {
        Self::new()
    }
}

/// Hook to access toast state from context
pub fn use_toast() -> ToastState {
    expect_context::<ToastState>()
}

/// Container component that renders active toasts.
#[component]
pub fn ToastContainer() -> impl IntoView {
    let toast_state = use_toast();

    view! {
        <div class="sc-toast-container">
            <For
                each=move || toast_state.toasts.get()
                key=|t| t.id
                let:toast
            >
                {
                    let level_class = toast.level.css_class();
                    let id = toast.id;
                    view! {
                        <div class={format!("sc-toast {}", level_class)}>
                            <span class="sc-toast-message">{toast.message.clone()}</span>
                            <button
                                class="sc-toast-dismiss"
                                on:click=move |_| toast_state.dismiss(id)
                            >
                                "×"
                            </button>
                        </div>
                    }
                }
            </For>
        </div>
    }
}

pub const TOAST_STYLES: &str = r#"
.sc-toast-container {
    position: fixed;
    top: 1rem;
    right: 1rem;
    z-index: 9999;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    pointer-events: none;
}
.sc-toast {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    font-family: var(--font-body);
    font-size: 0.9rem;
    color: var(--text-bright);
    pointer-events: auto;
    animation: sc-toast-in 0.3s ease;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
}
.sc-toast-info { border-left: 3px solid var(--info); }
.sc-toast-success { border-left: 3px solid var(--success); }
.sc-toast-warning { border-left: 3px solid var(--warning); }
.sc-toast-error { border-left: 3px solid var(--danger); }
.sc-toast-dismiss {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 1.2rem;
    padding: 0 0.25rem;
}
.sc-toast-dismiss:hover { color: var(--text-bright); }
@keyframes sc-toast-in {
    from { opacity: 0; transform: translateX(1rem); }
    to { opacity: 1; transform: translateX(0); }
}
"#;

use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub level: ToastLevel,
}

impl Toast {
    pub fn success(msg: impl Into<String>) -> Self {
        Self {
            id: 0,
            message: msg.into(),
            level: ToastLevel::Success,
        }
    }
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            id: 0,
            message: msg.into(),
            level: ToastLevel::Error,
        }
    }
    pub fn info(msg: impl Into<String>) -> Self {
        Self {
            id: 0,
            message: msg.into(),
            level: ToastLevel::Info,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ToastState {
    pub toasts: Signal<Vec<Toast>>,
    next_id: Signal<u64>,
}

impl ToastState {
    pub fn show(&mut self, mut toast: Toast) {
        let id = (self.next_id)();
        *self.next_id.write() += 1;
        toast.id = id;
        self.toasts.write().push(toast);

        // Auto-dismiss after 3 seconds
        let mut toasts = self.toasts;
        spawn(async move {
            #[cfg(feature = "web")]
            {
                gloo_timers::future::TimeoutFuture::new(3_000).await;
            }
            #[cfg(not(feature = "web"))]
            {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            toasts.write().retain(|t| t.id != id);
        });
    }

    pub fn dismiss(&mut self, id: u64) {
        self.toasts.write().retain(|t| t.id != id);
    }
}

pub fn use_toast() -> ToastState {
    use_context::<ToastState>()
}

const TOAST_CSS: &str = r#"
    .toast-container {
        position: fixed;
        top: 1rem;
        right: 1rem;
        z-index: 9999;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        pointer-events: none;
    }
    .toast {
        pointer-events: auto;
        display: flex;
        align-items: center;
        gap: 0.75rem;
        padding: 0.75rem 1rem;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: var(--bg-elevated);
        color: var(--text-bright);
        font-size: 0.85rem;
        min-width: 250px;
        animation: toast-in 0.3s ease-out;
    }
    .toast-success { border-left: 3px solid #34d399; }
    .toast-error { border-left: 3px solid #f87171; }
    .toast-warning { border-left: 3px solid #fbbf24; }
    .toast-info { border-left: 3px solid #60a5fa; }
    .toast-dismiss {
        margin-left: auto;
        background: none;
        border: none;
        color: var(--text-muted);
        cursor: pointer;
        font-size: 1rem;
        padding: 0 0.25rem;
    }
    .toast-dismiss:hover { color: var(--text-bright); }
    @keyframes toast-in {
        from { opacity: 0; transform: translateX(1rem); }
        to { opacity: 1; transform: translateX(0); }
    }
"#;

#[component]
pub fn ToastProvider(children: Element) -> Element {
    let toasts = use_signal(Vec::<Toast>::new);
    let next_id = use_signal(|| 1u64);
    let state = ToastState { toasts, next_id };
    use_context_provider(|| state);

    rsx! {
        style { {TOAST_CSS} }
        {children}
        div { class: "toast-container",
            for toast in toasts.read().iter() {
                { let id = toast.id;
                  let level_class = match toast.level {
                      ToastLevel::Success => "toast toast-success",
                      ToastLevel::Error => "toast toast-error",
                      ToastLevel::Warning => "toast toast-warning",
                      ToastLevel::Info => "toast toast-info",
                  };
                  let msg = toast.message.clone();
                  rsx! {
                      div { class: "{level_class}", key: "{id}",
                          span { "{msg}" }
                          button {
                              class: "toast-dismiss",
                              onclick: move |_| {
                                  let mut s = use_toast();
                                  s.dismiss(id);
                              },
                              "\u{00d7}"
                          }
                      }
                  }
                }
            }
        }
    }
}

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

/// ARIA role for one toast. Errors interrupt (`alert`); the rest wait their
/// turn (`status`). The container is `aria-live="polite"` so every save,
/// delete and error reaches screen readers (APP6).
fn toast_role(level: ToastLevel) -> &'static str {
    match level {
        ToastLevel::Error => "alert",
        ToastLevel::Info | ToastLevel::Success | ToastLevel::Warning => "status",
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
        background: var(--surface-2);
        color: var(--text);
        font-size: 0.85rem;
        min-width: 250px;
        animation: toast-in 0.3s ease-out;
    }
    .toast-success { border-left: 3px solid var(--ok); }
    .toast-error { border-left: 3px solid var(--danger); }
    .toast-warning { border-left: 3px solid var(--warn); }
    .toast-info { border-left: 3px solid var(--accent); }
    .toast-dismiss {
        margin-left: auto;
        background: none;
        border: none;
        color: var(--text-3);
        cursor: pointer;
        font-size: 1rem;
        padding: 0 0.25rem;
    }
    .toast-dismiss:hover { color: var(--text); }
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
        crate::styles::StyleOnce { id: "scuffed-css-components-toast-toast-css", css: TOAST_CSS }
        {children}
        div { class: "toast-container", aria_live: "polite",
            for toast in toasts.read().iter() {
                { let id = toast.id;
                  let level_class = match toast.level {
                      ToastLevel::Success => "toast toast-success",
                      ToastLevel::Error => "toast toast-error",
                      ToastLevel::Warning => "toast toast-warning",
                      ToastLevel::Info => "toast toast-info",
                  };
                  let role = toast_role(toast.level);
                  let msg = toast.message.clone();
                  rsx! {
                      div { class: "{level_class}", key: "{id}", role: "{role}",
                          span { "{msg}" }
                          button {
                              class: "toast-dismiss",
                              r#type: "button",
                              aria_label: "Dismiss",
                              onclick: move |_| {
                                  let mut s = state;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_alert_and_the_rest_are_status() {
        assert_eq!(toast_role(ToastLevel::Error), "alert");
        for level in [ToastLevel::Info, ToastLevel::Success, ToastLevel::Warning] {
            assert_eq!(toast_role(level), "status");
        }
    }

    /// APP6: the container is a polite live region, an error toast is an
    /// alert, and the dismiss button has a name.
    #[test]
    fn toasts_announce_to_screen_readers() {
        #[component]
        fn Seed() -> Element {
            // Push straight into the list. `show` would spawn the dismiss timer.
            let state = use_toast();
            use_hook(move || {
                let mut toasts = state.toasts;
                let mut ok = Toast::success("Settings saved.");
                ok.id = 1;
                let mut err = Toast::error("Failed to save settings: offline");
                err.id = 2;
                toasts.write().extend([ok, err]);
            });
            rsx! {}
        }

        fn view() -> Element {
            rsx! {
                ToastProvider { Seed {} }
            }
        }

        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        for _ in 0..4 {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("class=\"toast-container\" aria-live=\"polite\""),
            "{html}"
        );
        assert!(
            html.contains("class=\"toast toast-success\" role=\"status\""),
            "{html}"
        );
        assert!(
            html.contains("class=\"toast toast-error\" role=\"alert\""),
            "{html}"
        );
        assert_eq!(html.matches("aria-label=\"Dismiss\"").count(), 2, "{html}");
        assert!(html.contains("Failed to save settings: offline"), "{html}");
    }
}

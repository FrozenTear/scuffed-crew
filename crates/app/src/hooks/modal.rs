use dioxus::prelude::*;

/// Controls the lifecycle of a modal that operates on a target of type T.
///
/// Replaces the common pattern of 3 signals per modal:
///   let mut open = use_signal(|| false);
///   let mut target: Signal<Option<T>> = use_signal(|| None);
///   let mut submitting = use_signal(|| false);
pub struct ModalController<T: Clone + 'static> {
    pub open: Signal<bool>,
    pub target: Signal<Option<T>>,
    pub submitting: Signal<bool>,
}

// Manual Clone/Copy impls because derive requires T: Copy,
// but Signal<Option<T>> is Copy for all T: 'static.
impl<T: Clone + 'static> Clone for ModalController<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + 'static> Copy for ModalController<T> {}

impl<T: Clone + 'static> ModalController<T> {
    /// Create a new modal controller. Call this in your component body (it uses hooks).
    pub fn new() -> Self {
        Self {
            open: use_signal(|| false),
            target: use_signal(|| None),
            submitting: use_signal(|| false),
        }
    }

    /// Open the modal targeting a specific item (for edit/delete).
    pub fn show(&mut self, item: T) {
        self.target.set(Some(item));
        self.open.set(true);
    }

    /// Open the modal with no target (for create).
    pub fn show_empty(&mut self) {
        self.target.set(None);
        self.open.set(true);
    }

    /// Close the modal and reset state.
    pub fn close(&mut self) {
        self.open.set(false);
        self.submitting.set(false);
    }

    /// Check if the modal is currently open.
    pub fn is_open(&self) -> bool {
        (self.open)()
    }

    /// Check if the modal is currently submitting.
    pub fn is_submitting(&self) -> bool {
        (self.submitting)()
    }

    /// Get the current target (if any).
    pub fn get_target(&self) -> Option<T> {
        (self.target)()
    }

    /// Mark as submitting.
    pub fn start_submit(&mut self) {
        self.submitting.set(true);
    }

    /// Mark as done submitting (but don't close -- caller decides).
    pub fn end_submit(&mut self) {
        self.submitting.set(false);
    }
}

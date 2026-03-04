//! Undo/redo system using the command pattern.
//!
//! Plain Rust struct — no Dioxus signals. The component layer wraps
//! `UndoManager` in a `Signal` and calls `undo()`/`redo()` from event handlers.

use scuffed_types::strategy::{HeroId, HeroSelection, StrategyElement, TeamSlot, TimelinePhase};
use uuid::Uuid;

/// Maximum number of undo actions to keep.
const MAX_HISTORY: usize = 50;

// =============================================================================
// Undoable Actions
// =============================================================================

/// An action that can be reversed (undo) or reapplied (redo).
#[derive(Debug, Clone)]
pub enum UndoableAction {
    /// An element was added to the canvas.
    AddElement {
        element: StrategyElement,
    },
    /// An element was removed from the canvas (stores the element and its
    /// original index so it can be re-inserted in the right place).
    RemoveElement {
        element: StrategyElement,
        index: usize,
    },
    /// An element was modified (stores before and after snapshots).
    UpdateElement {
        id: Uuid,
        before: StrategyElement,
        after: StrategyElement,
    },
    /// A timeline phase was added.
    AddPhase {
        phase: TimelinePhase,
    },
    /// A timeline phase was removed.
    RemovePhase {
        phase: TimelinePhase,
        index: usize,
    },
    /// A hero was assigned to a team slot.
    AssignHeroToSlot {
        slot: TeamSlot,
        hero_id: HeroId,
        previous: Option<HeroId>,
    },
    /// A team slot was cleared.
    ClearSlot {
        slot: TeamSlot,
        previous_hero_id: HeroId,
    },
    /// All team slots were cleared at once.
    ClearAllSlots {
        previous: Vec<HeroSelection>,
    },
}

// =============================================================================
// Undo Manager
// =============================================================================

/// Manages undo and redo stacks.
#[derive(Debug, Clone)]
pub struct UndoManager {
    undo_stack: Vec<UndoableAction>,
    redo_stack: Vec<UndoableAction>,
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoManager {
    /// Create a new, empty undo manager.
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Record a new action. This pushes it onto the undo stack and clears the
    /// redo stack (because the timeline has diverged).
    pub fn push(&mut self, action: UndoableAction) {
        self.redo_stack.clear();
        self.undo_stack.push(action);

        // Trim oldest entries if we exceed the limit.
        while self.undo_stack.len() > MAX_HISTORY {
            self.undo_stack.remove(0);
        }
    }

    /// Pop the most recent action from the undo stack and push it onto the redo
    /// stack. Returns the action so the caller can reverse it.
    pub fn undo(&mut self) -> Option<UndoableAction> {
        let action = self.undo_stack.pop()?;
        self.redo_stack.push(action.clone());
        Some(action)
    }

    /// Pop the most recent action from the redo stack and push it back onto the
    /// undo stack. Returns the action so the caller can reapply it.
    pub fn redo(&mut self) -> Option<UndoableAction> {
        let action = self.redo_stack.pop()?;
        self.undo_stack.push(action.clone());
        Some(action)
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Discard all history (e.g. after loading a new strategy).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::strategy::{ElementType, Position};

    fn sample_element() -> StrategyElement {
        StrategyElement::new(ElementType::PlayerMarker, Position::new(0.0, 0.0))
    }

    #[test]
    fn push_and_undo() {
        let mut mgr = UndoManager::new();
        let elem = sample_element();

        mgr.push(UndoableAction::AddElement {
            element: elem.clone(),
        });
        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());

        let action = mgr.undo();
        assert!(action.is_some());
        assert!(!mgr.can_undo());
        assert!(mgr.can_redo());
    }

    #[test]
    fn redo_after_undo() {
        let mut mgr = UndoManager::new();
        let elem = sample_element();

        mgr.push(UndoableAction::AddElement {
            element: elem.clone(),
        });
        mgr.undo();

        let action = mgr.redo();
        assert!(action.is_some());
        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn new_action_clears_redo() {
        let mut mgr = UndoManager::new();
        let elem = sample_element();

        mgr.push(UndoableAction::AddElement {
            element: elem.clone(),
        });
        mgr.undo();
        assert!(mgr.can_redo());

        // New action should clear redo stack.
        mgr.push(UndoableAction::AddElement {
            element: sample_element(),
        });
        assert!(!mgr.can_redo());
    }

    #[test]
    fn max_history_enforced() {
        let mut mgr = UndoManager::new();

        for _ in 0..60 {
            mgr.push(UndoableAction::AddElement {
                element: sample_element(),
            });
        }

        assert_eq!(mgr.undo_stack.len(), MAX_HISTORY);
    }

    #[test]
    fn clear_empties_both_stacks() {
        let mut mgr = UndoManager::new();
        let elem = sample_element();

        mgr.push(UndoableAction::AddElement {
            element: elem.clone(),
        });
        mgr.push(UndoableAction::AddElement {
            element: sample_element(),
        });
        mgr.undo();

        assert!(mgr.can_undo());
        assert!(mgr.can_redo());

        mgr.clear();
        assert!(!mgr.can_undo());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn undo_on_empty_returns_none() {
        let mut mgr = UndoManager::new();
        assert!(mgr.undo().is_none());
    }

    #[test]
    fn redo_on_empty_returns_none() {
        let mut mgr = UndoManager::new();
        assert!(mgr.redo().is_none());
    }
}

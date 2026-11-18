pub const MAP_CANVAS_CSS: &str = r#"
    .map-canvas-wrapper {
        position: relative;
        width: 100%;
        height: 100%;
        overflow: hidden;
        background: var(--bg);
    }
    .map-canvas {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
    }
    .map-canvas-background {
        pointer-events: none;
        z-index: 1;
    }
    .map-canvas-elements {
        pointer-events: none;
        z-index: 2;
    }
    .map-canvas-overlay {
        z-index: 3;
    }
    /* The wrapper clips overflow, so the global outside ring would be cut
       off. Draw it inside the canvas edge. Focus comes by click (web and
       desktop), so :focus as well as :focus-visible, which the browser
       sets after the first key press. Two classes, so this beats the
       global `[tabindex]:focus-visible` ring (one attribute, one pseudo)
       whatever the order of the style sheets. */
    .map-canvas.map-canvas-overlay:focus,
    .map-canvas.map-canvas-overlay:focus-visible {
        outline: 2px solid var(--focus-ring);
        outline-offset: -2px;
    }
    .map-selector-overlay {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--overlay);
        z-index: 10;
    }
    .map-selector-content {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 2rem;
        max-width: 600px;
        width: 90%;
        max-height: 70vh;
        overflow-y: auto;
    }
    .map-selector-content h2 {
        font-family: var(--font-head);
        font-size: 1.5rem;
        color: var(--text);
        margin-bottom: 0.5rem;
    }
    .map-selector-content p {
        color: var(--text-3);
        margin-bottom: 1.5rem;
    }
    .map-selector-modes {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }
    .map-selector-mode-group {
        border: 1px solid var(--border);
        border-radius: 8px;
        overflow: hidden;
    }
    .map-selector-mode-title {
        padding: 0.6rem 1rem;
        background: var(--surface-sunken);
        color: var(--text);
        font-weight: 600;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: space-between;
    }
    .map-count {
        font-size: 0.8rem;
        color: var(--text-subtle);
        background: var(--surface);
        padding: 0.1rem 0.5rem;
        border-radius: 10px;
    }
    .map-selector-grid {
        display: flex;
        flex-direction: column;
    }
    .map-selector-btn,
    .map-selector-btn-submap {
        display: block;
        width: 100%;
        padding: 0.5rem 1rem;
        text-align: left;
        background: transparent;
        border: none;
        color: var(--text-3);
        cursor: pointer;
        transition: background 0.15s, color 0.15s;
    }
    .map-selector-btn:hover,
    .map-selector-btn-submap:hover {
        background: var(--surface-sunken);
        color: var(--text);
    }
    .map-selector-btn-submap {
        padding-left: 2rem;
        font-size: 0.9rem;
    }
    .map-selector-submap-group {
        border: none;
    }
    .map-selector-expand-hint {
        margin-left: 0.5rem;
        font-size: 0.8rem;
        color: var(--text-subtle);
    }
    .map-selector-submaps {
        padding-left: 0.5rem;
    }
"#;

#[cfg(test)]
mod tests {
    use super::MAP_CANVAS_CSS;

    /// Vision nit 3 on PR 693: the focused map canvas shows a ring inside its
    /// edge. The wrapper is `overflow: hidden`, so an outside ring is clipped.
    ///
    /// cargo test has no CSS engine, so this pins the rule text only; the
    /// computed ring (offset -2px after a click and again after a key press,
    /// when `:focus-visible` applies) is measured in a browser by `keys.mjs`.
    #[test]
    fn the_focused_canvas_ring_is_drawn_inside_the_clipping_wrapper() {
        let rule = MAP_CANVAS_CSS
            .split(".map-canvas.map-canvas-overlay:focus-visible {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("focus rule");
        assert!(
            MAP_CANVAS_CSS.contains(
                ".map-canvas.map-canvas-overlay:focus,\n    .map-canvas.map-canvas-overlay:focus-visible {"
            ),
            "the rule covers :focus and :focus-visible"
        );
        assert!(
            rule.contains("outline: 2px solid var(--focus-ring);"),
            "{rule}"
        );
        assert!(rule.contains("outline-offset: -2px;"), "{rule}");
        let wrapper = MAP_CANVAS_CSS
            .split(".map-canvas-wrapper {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("wrapper rule");
        assert!(wrapper.contains("overflow: hidden;"), "{wrapper}");
    }
}

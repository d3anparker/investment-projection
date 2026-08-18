//! The reactive editor-row model and the DOM helpers that operate on it.
//!
//! This is the one part of the split that genuinely needs leptos/`web_sys`
//! (signals, `NodeRef`, focus), so it is browser-verified rather than natively
//! tested — with one exception: the remove-button label's naming/fallback logic
//! is factored out as the pure [`label_for`], which *is* unit-tested.

use leptos::*;

/// One editable row. Every field is its own signal so typing in one cell does
/// not disturb the others. `Copy` because all fields are `Copy`.
#[derive(Clone, Copy)]
pub struct Row {
    pub id: usize,
    pub name: RwSignal<String>,
    pub value: RwSignal<String>,
    pub rate: RwSignal<String>,
    pub contribution: RwSignal<String>,
    /// The row's own remove button. Held here rather than created inside the
    /// `For` body so a *sibling* row's handler can reach it to place focus once
    /// this row is gone — see [`remove_row`].
    pub remove_btn: NodeRef<html::Button>,
}

pub fn new_row(
    counter: StoredValue<usize>,
    name: &str,
    value: &str,
    rate: &str,
    contribution: &str,
) -> Row {
    let id = counter.get_value();
    counter.set_value(id + 1);
    Row {
        id,
        name: create_rw_signal(name.to_string()),
        value: create_rw_signal(value.to_string()),
        rate: create_rw_signal(rate.to_string()),
        contribution: create_rw_signal(contribution.to_string()),
        remove_btn: create_node_ref(),
    }
}

/// Tooltip and accessible name for a row's remove button, given the holding's
/// name and its position in the list. Named rows get "Remove Global Equity
/// Fund"; the fallback numbers the row by position so several *unnamed* rows are
/// still told apart in a screen reader's element list — which matters more now
/// that focus lands on one of them after a removal.
pub fn label_for(name: &str, position: Option<usize>) -> String {
    let name = name.trim();
    if !name.is_empty() {
        return format!("Remove {name}");
    }
    match position {
        Some(i) => format!("Remove investment {}", i + 1),
        None => "Remove investment".to_string(),
    }
}

/// The live remove-button label for `r`. Reads both the name and the row's
/// position so the label updates when a holding is renamed or one above it is
/// removed; the pure naming logic lives in [`label_for`].
pub fn remove_label(r: Row, rows: RwSignal<Vec<Row>>) -> String {
    let name = r.name.get();
    let position = rows.with(|v| v.iter().position(|x| x.id == r.id));
    label_for(&name, position)
}

/// Remove the row with `id` and move focus somewhere sensible. Without this the
/// button the user just activated is torn out of the DOM and focus falls back to
/// `<body>`, dropping a keyboard user at the top of the page with the whole form
/// to tab through again.
///
/// Focus lands on the remove button of the row that slid into the vacated slot,
/// or the row above when the last one went, or `add_btn` when the list is empty.
/// No `request_animation_frame` needed: the `For` is keyed by `Row::id`, so every
/// surviving row keeps its DOM node and its `NodeRef` is already populated by the
/// time `update` returns.
pub fn remove_row(rows: RwSignal<Vec<Row>>, id: usize, add_btn: NodeRef<html::Button>) {
    let mut successor = None;
    rows.update(|v| {
        let Some(i) = v.iter().position(|x| x.id == id) else {
            return;
        };
        v.remove(i);
        successor = v.get(i).or_else(|| v.last()).copied();
    });
    match successor.and_then(|s| s.remove_btn.get_untracked()) {
        Some(btn) => {
            let _ = btn.focus();
        }
        None => {
            if let Some(btn) = add_btn.get_untracked() {
                let _ = btn.focus();
            }
        }
    }
}

/// Bind a text `<input>`'s value to `sig` without the caret-reset a plain
/// reactive `prop:value` causes. On every signal change Leptos would re-assign
/// `input.value`, which browsers treat as a fresh value and bounce the caret to
/// the end — disruptive when editing mid-string. Here the effect writes the DOM
/// value only when it actually differs from the signal, so ordinary typing
/// (where the DOM is already in sync, the edit having come *from* the input)
/// never triggers a write and the caret stays put; an external `sig.set(..)`
/// still updates the field.
pub fn bind_value(sig: RwSignal<String>) -> NodeRef<html::Input> {
    let node = create_node_ref::<html::Input>();
    create_effect(move |_| {
        let v = sig.get();
        // Tracked `get()` so the effect re-runs once the node mounts and applies
        // the initial value.
        if let Some(input) = node.get() {
            if input.value() != v {
                input.set_value(&v);
            }
        }
    });
    node
}

#[cfg(test)]
mod tests {
    use super::label_for;

    #[test]
    fn named_rows_read_remove_plus_name() {
        assert_eq!(label_for("Global Equity Fund", Some(0)), "Remove Global Equity Fund");
        // Surrounding whitespace is trimmed out of the label.
        assert_eq!(label_for("  Bonds  ", Some(3)), "Remove Bonds");
    }

    #[test]
    fn unnamed_rows_fall_back_to_a_one_based_position() {
        assert_eq!(label_for("", Some(0)), "Remove investment 1");
        assert_eq!(label_for("   ", Some(4)), "Remove investment 5");
    }

    #[test]
    fn unnamed_row_with_no_position_uses_the_bare_fallback() {
        assert_eq!(label_for("", None), "Remove investment");
    }
}

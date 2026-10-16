//! The shell the two modal sheets share: the glossary and the month-by-month
//! view. Each is a native `<dialog>` shown with `show_modal()`, styled by the
//! `.sheet` rules in `styles.css`; this is the markup half of that shell.
//!
//! # Focus goes back where it came from, and the platform does it
//!
//! `show_modal()` remembers what had focus and hands focus back to it on close,
//! so a keyboard user carries on from the button they pressed. That holds only
//! while the button still exists and actually had focus. Each sheet makes sure
//! of both at the source, so neither needs a fallback of its own:
//! - every trigger takes focus as it opens its sheet ([`focus_trigger`]),
//!   because Safari does not focus a clicked button;
//! - no trigger is rebuilt while its sheet is open. The form is inert behind the
//!   backdrop, and the comparison, which a debounce can still recompute under the
//!   month-by-month dialog, keys its rows so their buttons survive.

use leptos::*;

/// Focus `trigger` as it opens a sheet, before `show_modal()` is called.
///
/// `show_modal()` hands focus back on close to whatever had it on open. Safari
/// does not focus a button when it is clicked, so there it would remember the
/// body, and a keyboard user would be left at the top of the page. Focused
/// first, the trigger is what it remembers.
pub(crate) fn focus_trigger(trigger: NodeRef<html::Button>) {
    if let Some(button) = trigger.get_untracked() {
        let _ = button.focus();
    }
}

/// A sheet's header: its title, and a Close button that calls the dialog's own
/// `close()` rather than setting a signal. That keeps the dialog's `close` event
/// the single path back to shut, so `Esc` and the button cannot disagree.
pub(crate) fn sheet_head(
    heading_id: &'static str,
    title: &'static str,
    dialog: NodeRef<html::Dialog>,
) -> impl IntoView {
    let hide = move |_| {
        if let Some(d) = dialog.get_untracked() {
            d.close();
        }
    };
    view! {
        <div class="sheet-head">
            <h2 id=heading_id>{title}</h2>
            <button type="button" class="btn btn-ghost sheet-close" on:click=hide>
                "Close"
            </button>
        </div>
    }
}

//! The wrapper both output panels share.
//!
//! `summary` and `results` render different things but hold the same contract:
//! keep showing the last *good* projection through a transient error, dimmed via
//! `.stale` + `aria-busy` so it does not read as current, and fall back to the
//! placeholder only when the form is genuinely empty. That contract is an
//! accessibility one and easy to regress in one place and not the other, so it
//! lives here once rather than once per panel.

use calc::CalcOutput;
use leptos::*;

/// Wraps `body` in the last-good/`.stale` shell, or renders `empty` when there
/// is nothing to project.
///
/// Only `displayed` is read in the closure that builds the body. Staleness is
/// nothing but an opacity class and an ARIA flag, so it binds as a reactive
/// attribute instead: read here, the first invalid keystroke would rebuild the
/// entire panel — regenerating the chart SVG and the table, and taking the
/// scrubber's month and its keyboard focus with them — to dim it.
///
/// `body` takes the projection by reference so neither panel deep-clones a
/// `CalcOutput` (both per-month series, plus a row per holding) on every
/// recomputation; `results` clones only the two series it actually keeps.
pub fn stale_body<V, E>(
    displayed: Signal<Option<CalcOutput>>,
    stale: Signal<bool>,
    body: impl Fn(&CalcOutput) -> V + 'static,
    empty: impl Fn() -> E + 'static,
) -> impl IntoView
where
    V: IntoView,
    E: IntoView,
{
    move || {
        // Subscribe to the active currency so a jurisdiction switch re-renders
        // the figures even when the projection itself is unchanged (deposits
        // mode, where only the symbol differs). Reading it here, in the body
        // closure, is what wires that dependency in.
        let _ = crate::format::currency();
        displayed.with(|current| match current {
            Some(out) => view! {
                <div class="results-body" class:stale=move || stale.get()
                     aria-busy=move || stale.get().then_some("true")>
                    {body(out)}
                </div>
            }
            .into_view(),
            None => empty().into_view(),
        })
    }
}

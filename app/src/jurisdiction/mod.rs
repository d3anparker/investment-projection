//! The jurisdiction catalogue and its bespoke UI panels — the only place in
//! `app` that names a tax jurisdiction.
//!
//! Each [`Jurisdiction`] pairs a `taxkit::TaxSystem` with up to three optional
//! panel functions the page mounts at fixed points. A jurisdiction with no
//! bespoke controls (the UK) leaves them `None` and nothing renders, exactly as
//! before. Germany fills the settings and notes slots; the row slot stays unused
//! (see the note in `de-tax`'s catalogue on why a per-holding option would mean
//! a lifetime on `taxkit::Pot`).
//!
//! Panels are stored as `fn` pointers so the catalogue stays a `const`. A Leptos
//! 0.6 component returns an unnameable `impl IntoView`, so each function returns
//! a concrete [`leptos::View`] built from its own `view!`.
//!
//! **`app` names a jurisdiction only inside this module tree** (`convert`
//! reaches the active system through [`system_from`]); CI greps enforce it.

use leptos::*;
use std::collections::BTreeMap;

pub mod de;

/// A jurisdiction the app can project under.
pub struct Jurisdiction {
    /// Stable id, persisted in the share link.
    pub id: &'static str,
    /// Human name for the picker.
    pub label: &'static str,
    pub system: &'static dyn taxkit::TaxSystem,
    /// Portfolio-level controls, inside `.tax-settings`. `None` renders nothing.
    pub settings_panel: Option<fn(SettingsSlot) -> View>,
    /// Per-holding controls. Unused so far — kept for composability.
    pub row_panel: Option<fn(RowSlot) -> View>,
    /// Explanation of this jurisdiction's own figures, under the output panels.
    pub notes_panel: Option<fn(NotesSlot) -> View>,
}

/// What a settings panel is handed: the reactive option map it reads and writes,
/// and the current calendar year for any "year" default. Passing one bundle
/// means adding a control never changes the shared signature.
#[derive(Clone, Copy)]
pub struct SettingsSlot {
    pub options: RwSignal<BTreeMap<String, String>>,
    pub today_year: u16,
}

/// What a per-holding panel would be handed. Defined for composability; nothing
/// fills the slot yet.
#[derive(Clone, Copy)]
pub struct RowSlot {
    pub row_id: usize,
    pub options: RwSignal<BTreeMap<String, String>>,
}

/// What a notes panel is handed. Currently nothing — the German notes are
/// static explanatory text — but a struct leaves room to pass the projection in.
#[derive(Clone, Copy)]
pub struct NotesSlot;

/// Every jurisdiction the app offers, in picker order. The first is the default.
pub const JURISDICTIONS: &[Jurisdiction] = &[
    Jurisdiction {
        id: "uk",
        label: "United Kingdom",
        system: &uk_tax::UK,
        settings_panel: None,
        row_panel: None,
        notes_panel: None,
    },
    Jurisdiction {
        id: "de",
        label: "Germany",
        system: &de_tax::DE,
        settings_panel: Some(de::settings),
        row_panel: None,
        notes_panel: Some(de::notes),
    },
];

/// The id a blank/unknown jurisdiction falls back to — the first advertised one.
pub fn default_id() -> &'static str {
    JURISDICTIONS[0].id
}

/// Look up a jurisdiction by id, or the default for an unknown id (so a link
/// written under a jurisdiction this build does not have still opens).
pub fn from_id(id: &str) -> &'static Jurisdiction {
    JURISDICTIONS
        .iter()
        .find(|j| j.id == id)
        .unwrap_or(&JURISDICTIONS[0])
}

/// The active tax system for a jurisdiction id, permissively resolved.
pub fn system_from(id: &str) -> &'static dyn taxkit::TaxSystem {
    from_id(id).system
}

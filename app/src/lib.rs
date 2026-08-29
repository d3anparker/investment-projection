//! Leptos (CSR) front end for the investment projection tool.
//!
//! This layer owns the reactive form state and *formats* the `Decimal`s that
//! the `calc` crate returns. It performs no financial arithmetic itself — every
//! number is produced by `calc::calculate`. Its responsibilities are split into
//! focused modules:
//!
//! - [`convert`] — form strings → `calc::CalcInput` (blank-row filtering, the
//!   `<select>`→enum maps); pure and natively tested.
//! - [`outcome`] — a recomputation's result plus the error→control mapping;
//!   pure and natively tested.
//! - [`model`] — the reactive [`model::Row`] and its DOM helpers (signals, focus).
//! - [`summary`] / [`results`] — the two output panels' views, both wrapped by
//!   [`panel`]'s shared last-good/`.stale` shell.
//! - [`strategy`] — the side-by-side comparison of withdrawal orders, which
//!   reports several axes and deliberately ranks none of them.
//! - [`freshness`] — whether the tax figures are still current. Reads the clock
//!   here, at the edge, so `calc` stays a pure function of the form.
//! - [`format`] / [`chart`] — `Decimal`→string formatting and the SVG chart.
//! - [`app`] — the top-level [`App`] that wires these together, plus the mount
//!   entry (`main.rs`) and the browser-bound clipboard/history glue.
//!
//! Split into a library (this) plus a thin `main.rs` binary so the whole `App`
//! is reachable from the headless-browser suite in `app/tests/ui.rs`.

mod app;
pub mod chart;
pub mod convert;
pub mod format;
pub mod freshness;
pub mod goal;
pub mod model;
pub mod outcome;
pub mod panel;
pub mod results;
pub mod share;
pub mod strategy;
pub mod summary;

pub use app::App;

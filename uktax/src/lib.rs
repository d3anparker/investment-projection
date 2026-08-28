//! United Kingdom tax rules, as a [`taxkit::TaxSystem`].
//!
//! The crate is deliberately split in two, and the split is the point:
//!
//! * [`tables`] is **data** -- rates, thresholds, allowances and the account
//!   catalogue, all as integer literals. This is what an annual rate update
//!   edits, and nothing else.
//! * [`engine`] is **mechanism** -- how those figures become a schedule and how
//!   a withdrawal is priced against it. This changes only when the tax *system*
//!   changes: a new band structure, a different taper, a new kind of account.
//!
//! Consumers see neither. They hold a [`taxkit::TaxSystem`] and ask it what a
//! withdrawal costs, so swapping in another jurisdiction is a new crate and a
//! one-line change rather than a rewrite.
//!
//! ```no_run
//! # use taxkit::TaxSystem;
//! let system: &dyn TaxSystem = &uktax::UK;
//! assert_eq!(system.label(), "United Kingdom");
//! ```
//!
//! # What is and is not modelled
//!
//! Modelled: income tax across all four UK jurisdictions including the
//! withdrawn personal allowance; capital gains with the annual exempt amount and
//! both rates; phased tax-free pension cash against the lifetime lump sum
//! allowance; the normal minimum pension age.
//!
//! Not modelled: dividend and savings income (the projection has no concept of
//! yield, only total return), capital losses and their carry-forward,
//! chargeable event gains on investment bonds, inheritance tax, and the timing
//! of the state pension. Thresholds are frozen unless a caller asks for
//! uprating. These are simplifications, not oversights -- see the notes on the
//! individual account kinds in [`tables::UK_ACCOUNTS`].
//!
//! None of this is tax advice.

#![forbid(unsafe_code)]

pub mod engine;
pub mod tables;

pub use engine::{UkTaxSystem, UK};
pub use tables::{
    tax_year_label, uk_tax_year_of, Band, TaxJurisdiction, TaxYear, WithdrawalTax, LATEST,
    TAX_YEARS,
};

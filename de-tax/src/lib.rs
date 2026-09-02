//! Germany tax rules, as a [`taxkit::TaxSystem`].
//!
//! Split the same way `uk-tax` is, and for the same reason:
//!
//! * [`tables`] is **data** -- the §32a tariff coefficients, surcharge rates,
//!   allowances and the account catalogue, all as integer literals (euros as
//!   whole `i64`, rates as basis-point `u32`, tariff coefficients as `i64`
//!   *hundredths* of a euro). A yearly update edits only this file.
//! * [`tarif`] is **mechanism** for the one thing `taxkit::ladder` cannot walk:
//!   §32a is a *continuously progressive* tariff (quadratic within its middle
//!   zones), so its marginal rate rises smoothly rather than in constant steps.
//!   It is a private walker, so `taxkit` and `uk-tax` are untouched by it.
//! * [`engine`] is **mechanism** for everything else: the trait impls, and the
//!   dispatch between the flat-rate `taxkit::Ladder` (Abgeltungsteuer) and the
//!   progressive [`tarif::Tarif`] (pension income).
//!
//! ```no_run
//! # use taxkit::TaxSystem;
//! let system: &dyn TaxSystem = &de_tax::DE;
//! assert_eq!(system.label(), "Germany");
//! ```
//!
//! # What is and is not modelled
//!
//! Modelled: the §32a income-tax tariff including the Solidaritätszuschlag (with
//! its Freigrenze and Milderungszone) and Kirchensteuer; the flat Abgeltungsteuer
//! on capital income with the Sparer-Pauschbetrag and fund Teilfreistellung; the
//! Vorabpauschale as a periodic charge; joint assessment (Splittingverfahren);
//! and the cohort-fixed Besteuerungsanteil of a Rürup pension.
//!
//! Not modelled, and each a deliberate simplification a tool "for curiosity, not
//! advice" can carry: the Vorabpauschale's realised-gain cap is approximated
//! during drawdown; social contributions (Kranken-/Pflegeversicherung) on
//! pension income -- these are not tax; the Günstigerprüfung (capital income is
//! always charged the flat rate, which over-taxes a holder whose personal rate
//! is lower -- see `engine::GermanSession::capital_rate`); capital losses
//! and their separate buckets (`taxkit::Pot` cannot express a loss); dividend and
//! interest income as such (the projection models total return only); and
//! inheritance tax. None of this is tax advice.

#![forbid(unsafe_code)]

pub mod engine;
pub mod glossary;
pub mod tables;
pub mod tarif;

pub use engine::{GermanTaxSystem, DE};
pub use tables::{de_tax_year_of, tax_year_label, Treatment, WithdrawalTax, LATEST, TAX_YEARS};

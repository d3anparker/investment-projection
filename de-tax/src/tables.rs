//! The German figures, as data.
//!
//! **This file is data. Keep logic out of it.** Every euro threshold is a whole
//! `i64`; every rate is basis points (`u32`); every §32a tariff coefficient is
//! an `i64` number of *hundredths of a euro* (so 914.51 is written `91_451`), so
//! a yearly update is unambiguously a change of integer literals. Mechanism --
//! how these become a tariff and how a withdrawal is priced -- lives in
//! [`super::tarif`] and [`super::engine`].
//!
//! A rate update adds a *new* [`TaxYear`] const and prepends it to [`TAX_YEARS`];
//! it never edits an existing one, since old tables are the only record of what
//! the figures used to be. Adding an [`AccountKind`] here is in scope only when
//! its taxation matches an existing [`WithdrawalTax`]; a genuinely new mechanism
//! is an [`super::engine`]/[`super::tarif`] change and a decision.

use rust_decimal::Decimal;
use taxkit::{AccountKind, SimpleDate};

// --- the shape of a year ----------------------------------------------------

/// Every figure needed to tax a withdrawal in one German tax year.
///
/// The German tax year is the calendar year, so there is no 6-April trap: a
/// date's tax year is simply its calendar year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaxYear {
    /// Display name, e.g. "2026".
    pub label: &'static str,
    /// 1 January of the year.
    pub starts: SimpleDate,
    /// When these figures were last checked against source.
    pub as_of: SimpleDate,
    /// Where they came from.
    pub source_note: &'static str,

    // --- §32a income-tax tariff (coefficients in hundredths of a euro) ------
    /// Top of zone 1 (the Grundfreibetrag): income up to here pays nothing.
    pub grundfreibetrag_eur: i64,
    /// Top of zone 2 (the first, steeper progression zone).
    pub zone2_top_eur: i64,
    /// Top of zone 3 (the second progression zone); zone 4's 42% starts above.
    pub zone3_top_eur: i64,
    /// Top of zone 4; zone 5's 45% ("Reichensteuer") starts above.
    pub zone4_top_eur: i64,
    /// Zone 2 is `(a·y + b)·y` with `y = (zvE − grundfreibetrag)/10000`.
    pub zone2_a_cents: i64,
    pub zone2_b_cents: i64,
    /// Zone 3 is `(a·z + b)·z + c` with `z = (zvE − zone2_top)/10000`.
    pub zone3_a_cents: i64,
    pub zone3_b_cents: i64,
    pub zone3_c_cents: i64,
    /// Zone 4 is `0.42·zvE − sub`, `sub` in hundredths.
    pub zone4_sub_cents: i64,
    /// Zone 5 is `0.45·zvE − sub`, `sub` in hundredths.
    pub zone5_sub_cents: i64,
    /// Flat marginal rate in zone 4 (basis points), i.e. 4200 = 42%.
    pub upper_rate_bp: u32,
    /// Flat marginal rate in zone 5 (basis points), i.e. 4500 = 45%.
    pub top_rate_bp: u32,

    // --- surcharges ---------------------------------------------------------
    /// Solidaritätszuschlag rate on the income tax (550 = 5.5%).
    pub soli_bp: u32,
    /// Income-tax amount (not income) below which no Soli is due, single.
    pub soli_freigrenze_eur: i64,
    /// Milderungszone cap: Soli ≤ this fraction of (tax − Freigrenze) (1190 = 11.9%).
    pub soli_milderung_bp: u32,
    /// Kirchensteuer rate on the income tax, `[lower, higher]` — 8% in Bayern
    /// and Baden-Württemberg, 9% elsewhere.
    pub kirchensteuer_bp: [u32; 2],

    // --- capital income (Abgeltungsteuer) -----------------------------------
    /// Flat capital-income rate (2500 = 25%).
    pub kapest_bp: u32,
    /// Sparer-Pauschbetrag: tax-free capital income per year, single.
    pub sparer_pauschbetrag_eur: i64,
    /// Vorabpauschale base rate (Basiszins), BMF, each January (320 = 3.20%).
    pub basiszins_bp: u32,
    /// The 70% factor applied to the Basisertrag (7000 = 70%).
    pub vorab_faktor_bp: u32,

    // --- pensions -----------------------------------------------------------
    /// Taxable share of a Rürup/statutory pension by the year the pension
    /// *starts* — a cohort table, fixed for life at the start year, ascending.
    /// Read against the drawdown's start year.
    pub besteuerungsanteil: &'static [(u16, u32)],
    /// Earliest access age for a Rürup (Basisrente) pension.
    pub min_age_ruerup: u8,
    /// Earliest access age for a bAV (occupational) pension.
    pub min_age_bav: u8,
    /// The Regelaltersgrenze. Recorded for the legend; not consumed.
    pub regelaltersgrenze: u8,
}

/// The German tax year a date falls in: the calendar year.
pub const fn de_tax_year_of(d: SimpleDate) -> u16 {
    d.year
}

/// "2026" for 2026. Allocates, so it is not `const`.
pub fn tax_year_label(year: u16) -> String {
    year.to_string()
}

/// Basis points as a fraction: `2500` is `0.25`.
///
/// Lives beside the data it decodes, because every rate in this file is stored
/// in basis points and both `engine` and `tarif` have to read them.
pub(crate) fn bp(b: u32) -> Decimal {
    Decimal::new(i64::from(b), 4)
}

// --- 2026 -------------------------------------------------------------------

/// Cohort Besteuerungsanteil around the present: a pension started in 2026 is
/// 84% taxable, rising 0.5 point a year (Wachstumschancengesetz slowdown).
const BESTEUERUNGSANTEIL_2026: &[(u16, u32)] = &[
    (2023, 8250),
    (2024, 8300),
    (2025, 8350),
    (2026, 8400),
    (2027, 8450),
    (2028, 8500),
    (2029, 8550),
    (2030, 8600),
];

pub const TY_2026: TaxYear = TaxYear {
    label: "2026",
    starts: SimpleDate::new(2026, 1, 1),
    as_of: SimpleDate::new(2026, 9, 1),
    source_note: "gesetze-im-internet.de §32a EStG (ab VZ 2026); §32d EStG; §20 InvStG \
                  (Teilfreistellung); BMF Basiszins 2026; Solidaritätszuschlag Freigrenze 2026.",

    grundfreibetrag_eur: 12_348,
    zone2_top_eur: 17_799,
    zone3_top_eur: 69_878,
    zone4_top_eur: 277_825,
    zone2_a_cents: 91_451,   // 914.51
    zone2_b_cents: 140_000,  // 1400
    zone3_a_cents: 17_310,   // 173.10
    zone3_b_cents: 239_700,  // 2397
    zone3_c_cents: 103_487,  // 1034.87
    zone4_sub_cents: 1_113_563, // 11135.63
    zone5_sub_cents: 1_947_038, // 19470.38
    upper_rate_bp: 4200,
    top_rate_bp: 4500,

    soli_bp: 550,
    soli_freigrenze_eur: 20_350,
    soli_milderung_bp: 1190,
    kirchensteuer_bp: [800, 900],

    kapest_bp: 2500,
    sparer_pauschbetrag_eur: 1_000,
    basiszins_bp: 320,
    vorab_faktor_bp: 7000,

    besteuerungsanteil: BESTEUERUNGSANTEIL_2026,
    min_age_ruerup: 62,
    min_age_bav: 62,
    regelaltersgrenze: 67,
};

/// Every table, newest first.
pub const TAX_YEARS: &[&TaxYear] = &[&TY_2026];

/// The table used for projections.
pub const LATEST: &TaxYear = &TY_2026;

impl TaxYear {
    /// The taxable share (basis points) of a pension started in `year`, clamped
    /// to the ends of the cohort table. Calendar knowledge about the table, so
    /// it lives here beside it.
    pub fn besteuerungsanteil_for(&self, year: u16) -> u32 {
        let rows = self.besteuerungsanteil;
        if let Some((_, first)) = rows.first() {
            if year <= rows[0].0 {
                return *first;
            }
        }
        let mut share = rows.last().map(|(_, s)| *s).unwrap_or(10_000);
        for (y, s) in rows {
            if year >= *y {
                share = *s;
            }
        }
        share
    }
}

// --- the account catalogue --------------------------------------------------

/// How a withdrawal from an account is taxed.
///
/// Only [`super::engine`] and [`super::tarif`] match on this, so adding a variant
/// is confined to this crate -- no consumer sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WithdrawalTax {
    /// Nothing is due on the way out (cash; interest is not modelled).
    None,
    /// Abgeltungsteuer on the gain, after the Sparer-Pauschbetrag.
    FlatCapital,
    /// §32a on `taxable_share_bp` of the whole payment (pension income).
    ProgressiveIncome,
    /// §32a on `taxable_share_bp` of the gain (the 12-year / age-62 rule).
    ProgressiveGain,
    /// Real, holdable, and not modelled here. Selectable so a portfolio can be
    /// described honestly; excluded from tax-ordered strategies.
    NotModelled,
}

/// How each account is taxed, plus the two per-kind facts the neutral
/// `AccountKind` has nowhere to carry. A struct rather than a bare `WithdrawalTax`
/// because Germany needs three things per kind where the UK needed one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Treatment {
    pub tax: WithdrawalTax,
    /// Age below which this kind cannot normally be accessed. 0 = no gate.
    pub min_age: u8,
    /// Taxable share of the base, basis points (10000 = wholly taxable). `0`
    /// means "read the cohort table" and is valid only for `ProgressiveIncome`.
    pub taxable_share_bp: u32,
    /// Whether the periodic Vorabpauschale charge applies to this kind.
    pub vorabpauschale: bool,
}

impl Treatment {
    /// Whether this treatment taxes a *gain* (and so a cost basis is meaningful).
    pub fn taxes_a_gain(&self) -> bool {
        matches!(self.tax, WithdrawalTax::FlatCapital | WithdrawalTax::ProgressiveGain)
    }
}

pub mod ids {
    pub const GIRO: &str = "giro";
    pub const DEPOT_AKTIEN: &str = "depot_aktien";
    pub const FONDS_AKTIEN: &str = "fonds_aktien";
    pub const FONDS_MISCH: &str = "fonds_misch";
    pub const FONDS_IMMO: &str = "fonds_immo";
    pub const PRIVATE_RV: &str = "private_rv";
    pub const PRIVATE_RV_12: &str = "private_rv_12";
    pub const RUERUP: &str = "ruerup";
    pub const BAV: &str = "bav";
    pub const RIESTER: &str = "riester";
    pub const IMMOBILIE: &str = "immobilie";
}

/// Every account kind, in presentation order. The untaxed kind (`giro`) is
/// first, because [`taxkit::TaxSystem::default_account_kind`] defaults to the
/// first advertised kind and a blank picker must resolve to something untaxed.
pub const DE_ACCOUNTS: &[AccountKind] = &[
    AccountKind {
        id: ids::GIRO,
        label: "Giro-/Tagesgeldkonto",
        short_label: "Giro",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 0,
        note: "Cash. Interest is not modelled, so withdrawals are untaxed here.",
    },
    AccountKind {
        id: ids::DEPOT_AKTIEN,
        label: "Aktien-/Anleihedepot",
        short_label: "Depot",
        needs_cost_basis: true,
        age_gated: false,
        modelled: true,
        rank: 10,
        note: "Abgeltungsteuer on the gain; losses are not carried.",
    },
    AccountKind {
        id: ids::FONDS_AKTIEN,
        label: "Aktienfonds / Aktien-ETF",
        short_label: "Aktienfonds",
        needs_cost_basis: true,
        age_gated: false,
        modelled: true,
        rank: 11,
        note: "30% Teilfreistellung, so 70% of the gain is taxable. Vorabpauschale applies.",
    },
    AccountKind {
        id: ids::FONDS_MISCH,
        label: "Mischfonds",
        short_label: "Mischfonds",
        needs_cost_basis: true,
        age_gated: false,
        modelled: true,
        rank: 12,
        note: "15% Teilfreistellung, so 85% of the gain is taxable. Vorabpauschale applies.",
    },
    AccountKind {
        id: ids::FONDS_IMMO,
        label: "Immobilienfonds",
        short_label: "Immo.-fonds",
        needs_cost_basis: true,
        age_gated: false,
        modelled: true,
        rank: 13,
        note: "60% Teilfreistellung, so 40% of the gain is taxable. Vorabpauschale applies.",
    },
    AccountKind {
        id: ids::PRIVATE_RV,
        label: "Private Rentenversicherung (sonst)",
        short_label: "Private RV",
        needs_cost_basis: true,
        age_gated: false,
        modelled: true,
        rank: 20,
        note: "Lump-sum outside the 12-year / age-62 rule: taxed like capital on the gain.",
    },
    AccountKind {
        id: ids::PRIVATE_RV_12,
        label: "Private Rentenversicherung (12 J. / ab 62)",
        short_label: "Private RV 12",
        needs_cost_basis: true,
        age_gated: true,
        modelled: true,
        rank: 21,
        note: "Held 12 years and taken from 62: only half the gain is taxable, at the personal rate.",
    },
    AccountKind {
        id: ids::RUERUP,
        label: "Rürup-Rente (Basisrente)",
        short_label: "Rürup",
        needs_cost_basis: false,
        age_gated: true,
        modelled: true,
        rank: 30,
        note: "Taxed as income at the cohort Besteuerungsanteil fixed by the year drawing starts. \
               Statutory pension income belongs in the 'other taxable income' box, not here.",
    },
    AccountKind {
        id: ids::BAV,
        label: "Betriebliche Altersvorsorge",
        short_label: "bAV",
        needs_cost_basis: false,
        age_gated: true,
        modelled: true,
        rank: 31,
        note: "Payments taxed in full as income.",
    },
    AccountKind {
        id: ids::RIESTER,
        label: "Riester-Rente",
        short_label: "Riester",
        needs_cost_basis: false,
        age_gated: true,
        modelled: false,
        rank: 32,
        note: "Not modelled: its subsidy/claw-back rules need inputs this tool does not have.",
    },
    AccountKind {
        id: ids::IMMOBILIE,
        label: "Immobilie (direkt)",
        short_label: "Immobilie",
        needs_cost_basis: false,
        age_gated: false,
        modelled: false,
        rank: 40,
        note: "Not modelled: the speculation period and rental taxation are out of scope.",
    },
];

/// How each account is taxed. A parallel table rather than fields on
/// [`AccountKind`], because the treatment is this crate's business and
/// `AccountKind` is the shared, jurisdiction-neutral shape.
pub const DE_TREATMENT: &[(&str, Treatment)] = &[
    (ids::GIRO, Treatment { tax: WithdrawalTax::None, min_age: 0, taxable_share_bp: 0, vorabpauschale: false }),
    (ids::DEPOT_AKTIEN, Treatment { tax: WithdrawalTax::FlatCapital, min_age: 0, taxable_share_bp: 10_000, vorabpauschale: false }),
    (ids::FONDS_AKTIEN, Treatment { tax: WithdrawalTax::FlatCapital, min_age: 0, taxable_share_bp: 7_000, vorabpauschale: true }),
    (ids::FONDS_MISCH, Treatment { tax: WithdrawalTax::FlatCapital, min_age: 0, taxable_share_bp: 8_500, vorabpauschale: true }),
    (ids::FONDS_IMMO, Treatment { tax: WithdrawalTax::FlatCapital, min_age: 0, taxable_share_bp: 4_000, vorabpauschale: true }),
    (ids::PRIVATE_RV, Treatment { tax: WithdrawalTax::FlatCapital, min_age: 0, taxable_share_bp: 10_000, vorabpauschale: false }),
    (ids::PRIVATE_RV_12, Treatment { tax: WithdrawalTax::ProgressiveGain, min_age: 62, taxable_share_bp: 5_000, vorabpauschale: false }),
    // `0` share => read the cohort table.
    (ids::RUERUP, Treatment { tax: WithdrawalTax::ProgressiveIncome, min_age: 62, taxable_share_bp: 0, vorabpauschale: false }),
    (ids::BAV, Treatment { tax: WithdrawalTax::ProgressiveIncome, min_age: 62, taxable_share_bp: 10_000, vorabpauschale: false }),
    (ids::RIESTER, Treatment { tax: WithdrawalTax::NotModelled, min_age: 62, taxable_share_bp: 0, vorabpauschale: false }),
    (ids::IMMOBILIE, Treatment { tax: WithdrawalTax::NotModelled, min_age: 0, taxable_share_bp: 0, vorabpauschale: false }),
];

pub fn treatment_of(id: &str) -> Option<Treatment> {
    DE_TREATMENT.iter().find(|(k, _)| *k == id).map(|(_, t)| *t)
}

/// The conventional order in which accounts are spent: cash first, then the
/// taxable depots and funds (whose gains are taxed on disposal anyway, and whose
/// Sparer-Pauschbetrag is use-it-or-lose-it), then the pensions last. A judgement
/// about the German system, which is why it lives here.
pub const DE_CONVENTIONAL_ORDER: &[&str] = &[
    ids::GIRO,
    ids::DEPOT_AKTIEN,
    ids::FONDS_AKTIEN,
    ids::FONDS_MISCH,
    ids::FONDS_IMMO,
    ids::PRIVATE_RV,
    ids::PRIVATE_RV_12,
    ids::RUERUP,
    ids::BAV,
    ids::RIESTER,
    ids::IMMOBILIE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_german_tax_year_is_the_calendar_year() {
        assert_eq!(de_tax_year_of(SimpleDate::new(2026, 1, 1)), 2026);
        assert_eq!(de_tax_year_of(SimpleDate::new(2026, 12, 31)), 2026);
        assert_eq!(tax_year_label(2026), "2026");
    }

    #[test]
    fn tables_are_newest_first_and_latest_is_the_newest() {
        assert_eq!(*TAX_YEARS[0], *LATEST);
        for w in TAX_YEARS.windows(2) {
            assert!(w[0].starts > w[1].starts, "tables must be newest-first");
        }
        for y in TAX_YEARS {
            assert!(y.as_of >= y.starts);
            assert!(!y.source_note.is_empty());
        }
    }

    #[test]
    fn every_account_has_exactly_one_treatment_and_one_place_in_the_order() {
        for k in DE_ACCOUNTS {
            assert!(treatment_of(k.id).is_some(), "'{}' has no treatment", k.id);
            assert_eq!(
                DE_CONVENTIONAL_ORDER.iter().filter(|o| **o == k.id).count(),
                1,
                "'{}' should appear exactly once in the order",
                k.id
            );
            assert_eq!(DE_ACCOUNTS.iter().filter(|o| o.id == k.id).count(), 1);
        }
        assert_eq!(DE_ACCOUNTS.len(), DE_TREATMENT.len());
        assert_eq!(DE_ACCOUNTS.len(), DE_CONVENTIONAL_ORDER.len());
    }

    #[test]
    fn a_cost_basis_is_asked_for_exactly_when_a_gain_is_taxed() {
        for k in DE_ACCOUNTS {
            let t = treatment_of(k.id).unwrap();
            assert_eq!(
                k.needs_cost_basis,
                t.taxes_a_gain(),
                "'{}': needs_cost_basis must match taxing a gain",
                k.id
            );
        }
    }

    #[test]
    fn age_gating_matches_a_nonzero_min_age() {
        for k in DE_ACCOUNTS {
            let t = treatment_of(k.id).unwrap();
            assert_eq!(k.age_gated, t.min_age > 0, "'{}': age_gated must match min_age", k.id);
        }
    }

    #[test]
    fn unmodelled_accounts_say_why_in_their_note() {
        for k in DE_ACCOUNTS {
            if !k.modelled {
                assert!(!k.note.is_empty(), "'{}' is unmodelled but has no note", k.id);
            }
        }
    }

    #[test]
    fn the_cohort_table_ascends_and_clamps() {
        let ty = LATEST;
        for w in ty.besteuerungsanteil.windows(2) {
            assert!(w[0].0 < w[1].0, "cohort years must ascend");
            assert!(w[0].1 <= w[1].1, "cohort shares must not fall");
        }
        // Clamped at both ends.
        assert_eq!(ty.besteuerungsanteil_for(1900), ty.besteuerungsanteil[0].1);
        assert_eq!(
            ty.besteuerungsanteil_for(3000),
            ty.besteuerungsanteil.last().unwrap().1
        );
        assert_eq!(ty.besteuerungsanteil_for(2026), 8400);
    }

    #[test]
    fn no_rate_reaches_one_hundred_percent() {
        let ty = LATEST;
        for r in [ty.upper_rate_bp, ty.top_rate_bp, ty.soli_bp, ty.kapest_bp] {
            assert!(r < 10_000, "a rate at or above 100% cannot be grossed up");
        }
        for r in ty.kirchensteuer_bp {
            assert!(r < 10_000);
        }
    }
}

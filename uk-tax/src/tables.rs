//! UK tax figures, as literals.
//!
//! **This file is data. Keep logic out of it.** Every threshold is a whole
//! number of pounds and every rate is basis points, so a table update is
//! unambiguously a change of integer literals -- no macro, no `Decimal`
//! constructor, nothing to get subtly wrong. [`super::engine`] converts to
//! `Decimal` once when it builds a schedule.
//!
//! # Updating
//!
//! Add a **new** [`TaxYear`] const and prepend it to [`TAX_YEARS`]; do not edit
//! an existing one. Old tables cost nothing and are the only record of what the
//! figures used to be. Set `as_of` to the day you checked, and put the URLs you
//! actually used in `source_note`.
//!
//! Adding an [`AccountKind`] to [`UK_ACCOUNTS`] is in scope here **only** when
//! its taxation matches an existing [`WithdrawalTax`]. A genuinely new
//! mechanism -- top-slicing relief, say -- is an `engine.rs` change and a
//! decision, not a table edit.

use taxkit::{AccountKind, SimpleDate};

// --- jurisdictions ----------------------------------------------------------

/// Whose income tax rates apply.
///
/// HMRC assigns this from the taxpayer's main address, and it governs
/// **non-savings, non-dividend income only** -- savings, dividends and capital
/// gains use UK-wide rates for everyone.
///
/// Not "UK vs Scotland": Scotland is in the UK, and Wales sets its own rates
/// (the Welsh Rates of Income Tax) even though it currently sets them equal to
/// England's. Modelling all four means the day Wales diverges is a literal
/// change here rather than a code change anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaxJurisdiction {
    England,
    Wales,
    Scotland,
    NorthernIreland,
}

impl TaxJurisdiction {
    pub const ID_ENGLAND: &'static str = "england";
    pub const ID_WALES: &'static str = "wales";
    pub const ID_SCOTLAND: &'static str = "scotland";
    pub const ID_NORTHERN_IRELAND: &'static str = "northern_ireland";

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            Self::ID_ENGLAND => Some(Self::England),
            Self::ID_WALES => Some(Self::Wales),
            Self::ID_SCOTLAND => Some(Self::Scotland),
            Self::ID_NORTHERN_IRELAND => Some(Self::NorthernIreland),
            _ => None,
        }
    }
}

// --- the shape of a year ----------------------------------------------------

/// One step of a statutory schedule: everything from `from_gbp` up to the next
/// entry's `from_gbp` is charged at `rate_bp`.
///
/// Thresholds here are measured on **taxable** income -- that is, after the
/// personal allowance -- matching how HMRC states them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub from_gbp: i64,
    pub rate_bp: u32,
}

/// Every figure needed to tax a withdrawal in one UK tax year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaxYear {
    /// Display name, e.g. "2026/27".
    pub label: &'static str,
    /// 6 April of the opening year.
    pub starts: SimpleDate,
    /// When these figures were last checked against source.
    pub as_of: SimpleDate,
    /// Where they came from.
    pub source_note: &'static str,

    // --- income tax ---------------------------------------------------------
    pub personal_allowance_gbp: i64,
    /// Income above which the personal allowance is withdrawn.
    pub pa_taper_threshold_gbp: i64,
    /// Pounds of income that cost one pound of allowance.
    pub pa_taper_divisor: i64,
    /// Earned-income bands per jurisdiction. Three of the four point at the
    /// same slice today; when one diverges that is a literal change here.
    pub bands_england: &'static [Band],
    pub bands_wales: &'static [Band],
    pub bands_scotland: &'static [Band],
    pub bands_northern_ireland: &'static [Band],
    /// UK-wide regardless of jurisdiction.
    pub bands_savings: &'static [Band],
    pub bands_dividends: &'static [Band],
    /// Top of the UK basic-rate band on taxable income -- the pivot between the
    /// two capital gains rates. UK-wide even for Scottish taxpayers.
    pub basic_rate_limit_gbp: i64,

    // --- allowances ---------------------------------------------------------
    pub dividend_allowance_gbp: i64,
    /// Personal savings allowance by band reached: [basic, higher, additional].
    pub psa_gbp: [i64; 3],
    /// The 0% starting rate band for savings income.
    pub savings_starting_rate_gbp: i64,

    // --- capital gains ------------------------------------------------------
    pub cgt_annual_exempt_gbp: i64,
    pub cgt_rate_basic_bp: u32,
    pub cgt_rate_higher_bp: u32,

    // --- pensions -----------------------------------------------------------
    /// Tax-free fraction of a pension withdrawal.
    pub pcls_bp: u32,
    /// Lifetime cap on tax-free pension cash.
    pub lump_sum_allowance_gbp: i64,
    /// Age at which a pension can normally first be accessed.
    pub normal_minimum_pension_age: u8,
    /// Money purchase annual allowance. Recorded for completeness; it limits
    /// contributions, which this projection does not model.
    pub mpaa_gbp: i64,

    // --- ISA subscription limits -------------------------------------------
    // Recorded for completeness and for the UI legend. They cap what can be
    // paid in, which this projection does not model.
    pub isa_allowance_gbp: i64,
    pub lifetime_isa_allowance_gbp: i64,
    pub junior_isa_allowance_gbp: i64,
}

impl TaxYear {
    /// The one accessor the engine uses, so the four-field spread above stays
    /// out of the schedule builder.
    pub fn bands_for(&self, j: TaxJurisdiction) -> &'static [Band] {
        match j {
            TaxJurisdiction::England => self.bands_england,
            TaxJurisdiction::Wales => self.bands_wales,
            TaxJurisdiction::Scotland => self.bands_scotland,
            TaxJurisdiction::NorthernIreland => self.bands_northern_ireland,
        }
    }
}

/// The UK tax year a date falls in, named by its opening year: 6 April 2026 to
/// 5 April 2027 is 2026.
///
/// Calendar knowledge about the tables, which is why it lives beside them.
pub const fn uk_tax_year_of(d: SimpleDate) -> u16 {
    if d.month > 4 || (d.month == 4 && d.day >= 6) {
        d.year
    } else {
        d.year - 1
    }
}

/// "2026/27" for 2026. Allocates, so it is not `const`.
pub fn tax_year_label(opening: u16) -> String {
    format!("{}/{:02}", opening, (opening + 1) % 100)
}

// --- 2026/27 ----------------------------------------------------------------

/// England, Wales and Northern Ireland share one schedule for 2026/27.
const RUK_BANDS_2026_27: &[Band] = &[
    Band { from_gbp: 0, rate_bp: 2000 },       // basic
    Band { from_gbp: 37_700, rate_bp: 4000 },  // higher
    Band { from_gbp: 112_570, rate_bp: 4500 }, // additional (125,140 gross, no allowance left)
];

/// Scotland's six bands. Thresholds are on taxable income, i.e. net of the
/// personal allowance, so they are the published gross figures less 12,570.
const SCOTLAND_BANDS_2026_27: &[Band] = &[
    Band { from_gbp: 0, rate_bp: 1900 },       // starter
    Band { from_gbp: 2_827, rate_bp: 2000 },   // basic
    Band { from_gbp: 14_921, rate_bp: 2100 },  // intermediate
    Band { from_gbp: 31_092, rate_bp: 4200 },  // higher
    Band { from_gbp: 62_430, rate_bp: 4500 },  // advanced
    Band { from_gbp: 112_570, rate_bp: 4800 }, // top
];

const DIVIDEND_BANDS_2026_27: &[Band] = &[
    Band { from_gbp: 0, rate_bp: 1075 },
    Band { from_gbp: 37_700, rate_bp: 3575 },
    Band { from_gbp: 112_570, rate_bp: 3935 },
];

pub const TY_2026_27: TaxYear = TaxYear {
    label: "2026/27",
    starts: SimpleDate::new(2026, 4, 6),
    as_of: SimpleDate::new(2026, 8, 28),
    source_note: "gov.uk/income-tax-rates; gov.scot Scottish Income Tax 2026-27 technical \
                  factsheet; HMRC Pensions Tax Manual PTM062100; rates confirmed after \
                  Autumn Budget 2025.",

    personal_allowance_gbp: 12_570,
    pa_taper_threshold_gbp: 100_000,
    pa_taper_divisor: 2,
    bands_england: RUK_BANDS_2026_27,
    bands_wales: RUK_BANDS_2026_27,
    bands_scotland: SCOTLAND_BANDS_2026_27,
    bands_northern_ireland: RUK_BANDS_2026_27,
    bands_savings: RUK_BANDS_2026_27,
    bands_dividends: DIVIDEND_BANDS_2026_27,
    basic_rate_limit_gbp: 37_700,

    dividend_allowance_gbp: 500,
    psa_gbp: [1_000, 500, 0],
    savings_starting_rate_gbp: 5_000,

    cgt_annual_exempt_gbp: 3_000,
    cgt_rate_basic_bp: 1800,
    cgt_rate_higher_bp: 2400,

    pcls_bp: 2500,
    lump_sum_allowance_gbp: 268_275,
    normal_minimum_pension_age: 55,
    mpaa_gbp: 10_000,

    isa_allowance_gbp: 20_000,
    lifetime_isa_allowance_gbp: 4_000,
    junior_isa_allowance_gbp: 9_000,
};

/// Every table, newest first.
pub const TAX_YEARS: &[&TaxYear] = &[&TY_2026_27];

/// The table used for projections.
pub const LATEST: &TaxYear = &TY_2026_27;

// --- the account catalogue --------------------------------------------------

/// How a withdrawal from an account is taxed.
///
/// Only [`super::engine`] matches on this, so adding a variant is a change
/// confined to this crate -- no consumer sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WithdrawalTax {
    /// Nothing is due on the way out.
    None,
    /// A fixed fraction is tax free and the rest is income, taken piecemeal
    /// (the "uncrystallised funds pension lump sum" shape).
    IncomeWithTaxFreeFraction,
    /// Wholly taxable as income.
    IncomeOnly,
    /// Only the gain is chargeable, and disposals are proportional.
    CapitalGains,
    /// Real, holdable, and not modelled here. Selectable so a portfolio can be
    /// described honestly; excluded from tax-ordered strategies.
    NotModelled,
}

pub mod ids {
    pub const STOCKS_ISA: &str = "stocks_isa";
    pub const CASH_ISA: &str = "cash_isa";
    pub const LIFETIME_ISA: &str = "lifetime_isa";
    pub const JUNIOR_ISA: &str = "junior_isa";
    pub const PREMIUM_BONDS: &str = "premium_bonds";
    pub const SIPP: &str = "sipp";
    pub const WORKPLACE_DC: &str = "workplace_dc";
    pub const DEFINED_BENEFIT: &str = "defined_benefit";
    pub const GIA: &str = "gia";
    pub const VCT_EIS: &str = "vct_eis";
    pub const ONSHORE_BOND: &str = "onshore_bond";
    pub const OFFSHORE_BOND: &str = "offshore_bond";
}

/// Everything a UK portfolio can be held in.
///
/// Several kinds collapse onto the same [`WithdrawalTax`], which is the point:
/// the catalogue stays complete and recognisable to a user while the engine
/// only ever handles a handful of treatments.
pub const UK_ACCOUNTS: &[AccountKind] = &[
    AccountKind {
        id: ids::STOCKS_ISA,
        label: "Stocks & Shares ISA",
        short_label: "S&S ISA",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 10,
        note: "",
    },
    AccountKind {
        id: ids::CASH_ISA,
        label: "Cash ISA",
        short_label: "Cash ISA",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 11,
        note: "From April 2027 the cash subscription limit falls to \u{a3}12,000 for under-65s.",
    },
    AccountKind {
        id: ids::LIFETIME_ISA,
        label: "Lifetime ISA",
        short_label: "LISA",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 12,
        note: "Withdrawals before 60 other than for a first home carry a 25% charge, \
               which this projection does not model.",
    },
    AccountKind {
        id: ids::JUNIOR_ISA,
        label: "Junior ISA",
        short_label: "JISA",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 13,
        note: "Cannot be accessed until the child turns 18.",
    },
    AccountKind {
        id: ids::PREMIUM_BONDS,
        label: "Premium Bonds",
        short_label: "Prem. Bonds",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 14,
        note: "Prizes are tax free, but are a lottery rather than a return.",
    },
    AccountKind {
        id: ids::GIA,
        label: "General investment account",
        short_label: "GIA",
        needs_cost_basis: true,
        age_gated: false,
        modelled: true,
        rank: 20,
        note: "Unwrapped. Only the gain is taxable, and losses are not carried.",
    },
    AccountKind {
        id: ids::VCT_EIS,
        label: "VCT or EIS holding",
        short_label: "VCT/EIS",
        needs_cost_basis: false,
        age_gated: false,
        modelled: true,
        rank: 21,
        note: "Disposals are exempt once the qualifying period is met, which is assumed here.",
    },
    AccountKind {
        id: ids::SIPP,
        label: "SIPP",
        short_label: "SIPP",
        needs_cost_basis: false,
        age_gated: true,
        modelled: true,
        rank: 30,
        note: "",
    },
    AccountKind {
        id: ids::WORKPLACE_DC,
        label: "Workplace pension (defined contribution)",
        short_label: "Workplace DC",
        needs_cost_basis: false,
        age_gated: true,
        modelled: true,
        rank: 31,
        note: "",
    },
    AccountKind {
        id: ids::DEFINED_BENEFIT,
        label: "Defined benefit pension",
        short_label: "DB pension",
        needs_cost_basis: false,
        age_gated: true,
        modelled: true,
        rank: 32,
        note: "Taxed wholly as income. A real DB pension pays a set amount rather than \
               drawing down a pot, so treat this as an approximation.",
    },
    AccountKind {
        id: ids::ONSHORE_BOND,
        label: "Onshore investment bond",
        short_label: "Onshore bond",
        needs_cost_basis: false,
        age_gated: false,
        modelled: false,
        rank: 40,
        note: "Chargeable event gains and top-slicing relief are not modelled.",
    },
    AccountKind {
        id: ids::OFFSHORE_BOND,
        label: "Offshore investment bond",
        short_label: "Offshore bond",
        needs_cost_basis: false,
        age_gated: false,
        modelled: false,
        rank: 41,
        note: "Chargeable event gains and top-slicing relief are not modelled.",
    },
];

/// How each account is taxed on the way out. A parallel table rather than a
/// field on [`AccountKind`], because the treatment is this crate's business and
/// `AccountKind` is the shared, jurisdiction-neutral shape.
pub const UK_TREATMENT: &[(&str, WithdrawalTax)] = &[
    (ids::STOCKS_ISA, WithdrawalTax::None),
    (ids::CASH_ISA, WithdrawalTax::None),
    (ids::LIFETIME_ISA, WithdrawalTax::None),
    (ids::JUNIOR_ISA, WithdrawalTax::None),
    (ids::PREMIUM_BONDS, WithdrawalTax::None),
    (ids::VCT_EIS, WithdrawalTax::None),
    (ids::GIA, WithdrawalTax::CapitalGains),
    (ids::SIPP, WithdrawalTax::IncomeWithTaxFreeFraction),
    (ids::WORKPLACE_DC, WithdrawalTax::IncomeWithTaxFreeFraction),
    (ids::DEFINED_BENEFIT, WithdrawalTax::IncomeOnly),
    (ids::ONSHORE_BOND, WithdrawalTax::NotModelled),
    (ids::OFFSHORE_BOND, WithdrawalTax::NotModelled),
];

pub fn treatment_of(id: &str) -> Option<WithdrawalTax> {
    UK_TREATMENT.iter().find(|(k, _)| *k == id).map(|(_, t)| *t)
}

/// The conventional order in which accounts are spent: unwrapped first (its
/// gains are taxed as they accrue anyway), then the ISAs, then pensions last.
///
/// This is a judgement about the UK system, which is exactly why it belongs
/// here rather than in a projection engine.
pub const UK_CONVENTIONAL_ORDER: &[&str] = &[
    ids::GIA,
    ids::VCT_EIS,
    ids::ONSHORE_BOND,
    ids::OFFSHORE_BOND,
    ids::PREMIUM_BONDS,
    ids::CASH_ISA,
    ids::STOCKS_ISA,
    ids::LIFETIME_ISA,
    ids::JUNIOR_ISA,
    ids::DEFINED_BENEFIT,
    ids::WORKPLACE_DC,
    ids::SIPP,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tax_year_turns_on_the_sixth_of_april() {
        assert_eq!(uk_tax_year_of(SimpleDate::new(2026, 4, 5)), 2025);
        assert_eq!(uk_tax_year_of(SimpleDate::new(2026, 4, 6)), 2026);
        assert_eq!(uk_tax_year_of(SimpleDate::new(2026, 12, 31)), 2026);
        assert_eq!(uk_tax_year_of(SimpleDate::new(2027, 1, 1)), 2026);
        assert_eq!(uk_tax_year_of(SimpleDate::new(2027, 3, 31)), 2026);
    }

    #[test]
    fn labels_pad_the_second_year() {
        assert_eq!(tax_year_label(2026), "2026/27");
        assert_eq!(tax_year_label(2099), "2099/00");
    }

    #[test]
    fn every_bands_slice_ascends_from_zero() {
        for year in TAX_YEARS {
            for bands in [
                year.bands_england,
                year.bands_wales,
                year.bands_scotland,
                year.bands_northern_ireland,
                year.bands_savings,
                year.bands_dividends,
            ] {
                assert!(!bands.is_empty(), "{}: a schedule cannot be empty", year.label);
                assert_eq!(bands[0].from_gbp, 0, "{}: schedules start at zero", year.label);
                for w in bands.windows(2) {
                    assert!(
                        w[1].from_gbp > w[0].from_gbp,
                        "{}: thresholds must strictly ascend",
                        year.label
                    );
                }
                for b in bands {
                    assert!(b.rate_bp < 10_000, "{}: a rate at or above 100% cannot be grossed up", year.label);
                }
            }
        }
    }

    #[test]
    fn the_taper_exactly_exhausts_the_allowance() {
        for year in TAX_YEARS {
            let spent = year.personal_allowance_gbp * year.pa_taper_divisor;
            let zero_at = year.pa_taper_threshold_gbp + spent;
            // The additional-rate threshold is defined as the point the
            // allowance runs out, so these must agree or the flattened schedule
            // the engine builds would have an inconsistent rung.
            let additional = year
                .bands_england
                .last()
                .expect("a schedule has a top band")
                .from_gbp
                + year.personal_allowance_gbp;
            assert_eq!(zero_at, additional, "{}: taper and top band disagree", year.label);
        }
    }

    #[test]
    fn the_capital_gains_pivot_matches_the_higher_rate_threshold() {
        for year in TAX_YEARS {
            // `basic_rate_limit_gbp` and the rUK higher-rate band start are the
            // same statutory figure written twice: one drives the capital gains
            // rate pivot, the other the income schedule. A rate update that
            // moves one and forgets the other is a wrong number with no other
            // symptom, so pin them together the way the taper test does.
            let higher_from = year
                .bands_england
                .get(1)
                .expect("a schedule has a higher band")
                .from_gbp;
            assert_eq!(
                year.basic_rate_limit_gbp, higher_from,
                "{}: the capital gains pivot and the higher-rate threshold disagree",
                year.label
            );
        }
    }

    #[test]
    fn tables_are_newest_first_and_latest_is_the_newest() {
        assert_eq!(*TAX_YEARS[0], *LATEST);
        for w in TAX_YEARS.windows(2) {
            assert!(w[0].starts > w[1].starts, "TAX_YEARS is newest first");
        }
        for year in TAX_YEARS {
            assert!(
                year.as_of >= year.starts,
                "{}: figures cannot have been checked before the year began",
                year.label
            );
            assert!(!year.source_note.is_empty(), "{}: say where it came from", year.label);
        }
    }

    #[test]
    fn every_account_has_exactly_one_treatment_and_one_place_in_the_order() {
        for k in UK_ACCOUNTS {
            assert!(
                treatment_of(k.id).is_some(),
                "'{}' has no withdrawal treatment",
                k.id
            );
            assert_eq!(
                UK_CONVENTIONAL_ORDER.iter().filter(|o| **o == k.id).count(),
                1,
                "'{}' should appear exactly once in the conventional order",
                k.id
            );
            assert_eq!(
                UK_ACCOUNTS.iter().filter(|o| o.id == k.id).count(),
                1,
                "'{}' is defined twice",
                k.id
            );
        }
        assert_eq!(UK_TREATMENT.len(), UK_ACCOUNTS.len());
        assert_eq!(UK_CONVENTIONAL_ORDER.len(), UK_ACCOUNTS.len());
    }

    #[test]
    fn only_gains_taxed_accounts_ask_for_a_cost_basis() {
        for k in UK_ACCOUNTS {
            let capital = treatment_of(k.id) == Some(WithdrawalTax::CapitalGains);
            assert_eq!(
                k.needs_cost_basis, capital,
                "'{}': the UI shows a cost basis exactly when the gain is what is taxed",
                k.id
            );
        }
    }

    #[test]
    fn pensions_and_only_pensions_are_age_gated() {
        for k in UK_ACCOUNTS {
            let pension = matches!(
                treatment_of(k.id),
                Some(WithdrawalTax::IncomeWithTaxFreeFraction) | Some(WithdrawalTax::IncomeOnly)
            );
            assert_eq!(k.age_gated, pension, "'{}': age gating tracks pension status", k.id);
        }
    }

    #[test]
    fn unmodelled_accounts_say_why_in_their_note() {
        for k in UK_ACCOUNTS {
            if !k.modelled {
                assert!(
                    !k.note.is_empty(),
                    "'{}' is not modelled, so the UI must be able to say so",
                    k.id
                );
            }
        }
    }

    #[test]
    fn jurisdiction_ids_round_trip() {
        for id in [
            TaxJurisdiction::ID_ENGLAND,
            TaxJurisdiction::ID_WALES,
            TaxJurisdiction::ID_SCOTLAND,
            TaxJurisdiction::ID_NORTHERN_IRELAND,
        ] {
            assert!(TaxJurisdiction::from_id(id).is_some(), "'{id}' should resolve");
        }
        assert!(TaxJurisdiction::from_id("narnia").is_none());
    }

    #[test]
    fn scotland_has_more_bands_than_the_rest_and_they_are_its_own() {
        let y = LATEST;
        assert!(y.bands_scotland.len() > y.bands_england.len());
        assert_eq!(y.bands_england, y.bands_wales);
        assert_eq!(y.bands_england, y.bands_northern_ireland);
        assert_ne!(y.bands_england, y.bands_scotland);
    }
}

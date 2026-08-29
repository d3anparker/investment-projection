//! Turning the raw form strings into a `calc::CalcInput`.
//!
//! This is the marshalling layer between the reactive UI and the pure `calc`
//! core: it applies the UI's own conventions (blank means zero, the `<select>`
//! string values map to `calc` enums) and — critically — filters out blank rows
//! *before* `calc` sees them. That filtering breaks the index correspondence
//! between `CalcInput::investments` and the rows on screen, so [`build_input`]
//! also returns the surviving rows' ids in order; the memo in `main.rs` pairs
//! that with a [`crate::outcome::Outcome`] so a `calc` error can be mapped back
//! to the right control.
//!
//! All of this is pure (no signals, no DOM), so it is unit-tested natively — the
//! [`FormInput`]/[`RowData`] snapshot the reactive form down to plain strings for
//! exactly that reason.

use calc::{CalcInput, InvestmentInput, Plan, Strategy, TaxContext, Unit};

/// The tax system this build of the app is wired to.
///
/// **The only place the app names a jurisdiction.** Every control that mentions
/// an account, a region or a currency is populated from this, so swapping the
/// line swaps the whole tax model and nothing else in `app` changes.
pub const TAX_SYSTEM: &dyn taxkit::TaxSystem = &uktax::UK;

/// A plain-string snapshot of one editor row, decoupled from the reactive
/// `Row`'s signals so the input-building logic can be tested without a runtime.
/// `Clone` so it can also carry a projection's form state into [`crate::share`].
#[derive(Clone, Default, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct RowData {
    // The id is positional bookkeeping for the reactive layer, not part of the
    // shared state: the codec drops it on the way out and reassigns it by row
    // position on the way back in, so it never bloats the link.
    #[serde(skip)]
    pub id: usize,
    pub name: String,
    pub value: String,
    pub rate: String,
    pub contribution: String,
    /// Which account kind the holding sits in, as an id from the active tax
    /// system's catalogue. `#[serde(default)]` so links written before accounts
    /// existed still decode — they come back blank, which resolves to the first
    /// (untaxed) kind and reproduces exactly what they used to show.
    #[serde(default)]
    pub account_kind: String,
    /// What the holding originally cost. Only shown, and only consulted, for
    /// account kinds whose `needs_cost_basis` is set.
    #[serde(default)]
    pub cost_basis: String,
}

/// A plain-string snapshot of the whole form, decoupled from the signals. The
/// single argument to [`build_input`], so adding a form-level control means one
/// new field here rather than another positional string parameter. Deliberately
/// *not* `serde` and deliberately without the goal fields: it feeds the
/// projection memo, whose dependency set must stay as narrow as it is — typing in
/// the goal box must not re-run `calculate`.
pub struct FormInput {
    pub rows: Vec<RowData>,
    pub horizon_value: String,
    pub horizon_unit: String,
    /// `"deposits"` or `"drawdown"` — the top-level mode.
    pub plan: String,
    pub drawdown_value: String,
    pub drawdown_unit: String,
    pub withdrawal: String,
    /// The withdrawal-order picker's value. Blank is pro-rata, which is what
    /// every link written before strategies existed decodes to.
    pub strategy: String,
    /// The rate cap that belongs to the rate-capped strategy, as a percent.
    pub rate_cap: String,
    /// Portfolio-level tax details. All blank means an untaxed projection.
    pub region: String,
    pub other_income: String,
    pub age: String,
    pub uprate: String,
}

/// Build the `calc` input from the current form, dropping blank rows. Returns the
/// input alongside the ids of the rows that survived the filter, in order, so a
/// `CalcError`'s index (into `CalcInput::investments`) can be mapped back to the
/// row on screen that caused it.
pub fn build_input(f: &FormInput) -> (CalcInput, Vec<usize>) {
    let mut row_ids = Vec::new();
    let investments: Vec<InvestmentInput> = f
        .rows
        .iter()
        .filter_map(|r| {
            // Skip blank rows so a half-typed row doesn't error the form. A row
            // counts as present if it has *any* of value/rate/contribution.
            if r.value.trim().is_empty()
                && r.rate.trim().is_empty()
                && r.contribution.trim().is_empty()
            {
                return None;
            }
            row_ids.push(r.id);
            Some(InvestmentInput {
                name: if r.name.trim().is_empty() {
                    "Investment".into()
                } else {
                    r.name.clone()
                },
                value: blank_zero(&r.value),
                rate: blank_zero(&r.rate),
                contribution: blank_zero(&r.contribution),
                account_kind: kind_from(&r.account_kind),
                // Left exactly as typed: `calc` reads a blank as "today's
                // value", which is not the same thing as zero.
                cost_basis: r.cost_basis.clone(),
            })
        })
        .collect();

    let input = CalcInput {
        investments,
        horizon_value: blank_zero(&f.horizon_value),
        horizon_unit: unit_from(&f.horizon_unit),
        plan: plan_from(f),
        tax: tax_from(f),
    };
    (input, row_ids)
}

/// A blank (or whitespace-only) field means "nothing here" — send `calc` a
/// literal `"0"` so it parses rather than erroring on an empty string.
pub fn blank_zero(s: &str) -> String {
    if s.trim().is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// A period unit `<select>`'s string value → `calc::Unit`. Anything other than
/// `"months"` is years (the default option).
pub fn unit_from(s: &str) -> Unit {
    if s == "months" {
        Unit::Months
    } else {
        Unit::Years
    }
}

/// The top-level mode → `calc::Plan`. Anything other than `"drawdown"` (including
/// a blank from a pre-mode shared link) is the deposits default, so the drawdown
/// period and withdrawal are only read when actually drawing down.
pub fn plan_from(f: &FormInput) -> Plan {
    if f.plan == "drawdown" {
        Plan::Drawdown {
            drawdown_value: blank_zero(&f.drawdown_value),
            drawdown_unit: unit_from(&f.drawdown_unit),
            // A blank withdrawal is a flat drawdown (zero), which `calc` accepts.
            withdrawal: blank_zero(&f.withdrawal),
            strategy: strategy_from(&f.strategy, &f.rate_cap),
        }
    } else {
        Plan::Deposits
    }
}

// --- the tax controls -------------------------------------------------------
//
// Every resolver below is deliberately permissive in the same way `unit_from`
// is: an unrecognised value falls back to the default rather than erroring, so a
// link from an older build, or one written against a different tax system,
// still projects instead of showing a validation message the reader cannot act
// on. Genuine mistakes the user *can* fix — a nonsensical amount, an
// out-of-range age — are `calc`'s business and come back as a `CalcError`.

/// Ids the strategy `<select>` uses. Kept here rather than inline in the markup
/// so the select, the share codec and the resolver cannot drift apart.
pub const STRATEGY_PRO_RATA: &str = "pro-rata";
pub const STRATEGY_CONVENTIONAL: &str = "conventional";
pub const STRATEGY_CHEAPEST: &str = "cheapest";
pub const STRATEGY_PRESERVE: &str = "preserve";
pub const STRATEGY_CAPPED: &str = "capped";

/// The withdrawal-order picker → `calc::Strategy`.
///
/// The conventional order is asked of the *tax system*, never written here:
/// which accounts are conventionally spent first follows from how they are
/// taxed, so it is jurisdiction-specific knowledge.
pub fn strategy_from(strategy: &str, rate_cap: &str) -> Strategy {
    match strategy {
        STRATEGY_CONVENTIONAL => Strategy::Ordered {
            order: TAX_SYSTEM
                .conventional_order()
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        },
        STRATEGY_CHEAPEST => Strategy::CheapestFirst,
        STRATEGY_PRESERVE => Strategy::PreserveGrowth,
        STRATEGY_CAPPED => Strategy::RateCapped { max_rate: blank_zero(rate_cap) },
        _ => Strategy::ProRata,
    }
}

/// An account-kind id, checked against the catalogue. An id the active system
/// does not advertise falls back to the first kind rather than erroring, so a
/// link built against a different catalogue still opens.
pub fn kind_from(id: &str) -> String {
    let id = id.trim();
    if TAX_SYSTEM.account_kind(id).is_some() {
        id.to_string()
    } else {
        TAX_SYSTEM.default_account_kind().map_or(String::new(), |k| k.id.to_string())
    }
}

/// A region id, checked the same way.
pub fn region_from(id: &str) -> String {
    let id = id.trim();
    if TAX_SYSTEM.region(id).is_some() {
        id.to_string()
    } else {
        TAX_SYSTEM.regions().first().map_or(String::new(), |r| r.id.to_string())
    }
}

/// The portfolio-level tax details, or `None` for an untaxed projection.
///
/// Only built while drawing down, and only for a strategy that is not pro-rata:
/// pro-rata ignores tax entirely, and handing it a context would be misleading
/// rather than merely wasteful — the output would advertise a tax year it never
/// used.
pub fn tax_from(f: &FormInput) -> Option<TaxContext> {
    if strategy_from(&f.strategy, &f.rate_cap) == Strategy::ProRata {
        return None;
    }
    tax_context(f)
}

/// The tax details the form describes, whatever order is currently selected.
///
/// [`tax_from`] withholds this from a pro-rata projection, because pro-rata
/// ignores tax and a context would make the output claim a tax year it never
/// used. The strategy comparison is the one caller for which that reasoning
/// inverts — its whole job is to show what the *other* orders would do, so it
/// asks for the context directly.
pub fn tax_context(f: &FormInput) -> Option<TaxContext> {
    if f.plan != "drawdown" {
        return None;
    }
    Some(TaxContext {
        system: TAX_SYSTEM,
        region: region_from(&f.region),
        other_income: f.other_income.clone(),
        age: f.age.clone(),
        uprate: f.uprate.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: usize, name: &str, value: &str, rate: &str, contribution: &str) -> RowData {
        RowData {
            id,
            name: name.into(),
            value: value.into(),
            rate: rate.into(),
            contribution: contribution.into(),
            ..Default::default()
        }
    }

    fn form(rows: Vec<RowData>, horizon_value: &str, horizon_unit: &str) -> FormInput {
        FormInput {
            rows,
            horizon_value: horizon_value.into(),
            horizon_unit: horizon_unit.into(),
            plan: "deposits".into(),
            drawdown_value: "30".into(),
            drawdown_unit: "years".into(),
            withdrawal: String::new(),
            strategy: String::new(),
            rate_cap: String::new(),
            region: String::new(),
            other_income: String::new(),
            age: String::new(),
            uprate: String::new(),
        }
    }

    /// A drawdown form, for the plan/tax resolvers.
    fn drawing(withdrawal: &str, strategy: &str) -> FormInput {
        FormInput {
            plan: "drawdown".into(),
            withdrawal: withdrawal.into(),
            strategy: strategy.into(),
            ..form(vec![], "10", "years")
        }
    }

    #[test]
    fn blank_zero_maps_empty_and_whitespace_to_zero() {
        assert_eq!(blank_zero(""), "0");
        assert_eq!(blank_zero("   "), "0");
        assert_eq!(blank_zero("\t"), "0");
        // Anything non-blank passes straight through, untrimmed.
        assert_eq!(blank_zero("10000"), "10000");
        assert_eq!(blank_zero(" 10,000 "), " 10,000 ");
    }

    #[test]
    fn unit_from_defaults_to_years() {
        // `calc::Unit` compares with `==`.
        assert!(unit_from("months") == Unit::Months);
        assert!(unit_from("years") == Unit::Years);
        assert!(unit_from("") == Unit::Years);
        assert!(unit_from("decades") == Unit::Years);
    }

    #[test]
    fn plan_from_defaults_to_deposits() {
        assert!(plan_from(&form(vec![], "10", "years")) == Plan::Deposits);
        // Blank (a pre-mode shared link) and anything unknown are deposits.
        for mode in ["", "nonsense"] {
            let f = FormInput { plan: mode.into(), ..form(vec![], "10", "years") };
            assert!(plan_from(&f) == Plan::Deposits, "{mode}");
        }
    }

    #[test]
    fn plan_from_carries_the_drawdown_fields() {
        let p = plan_from(&drawing("2000", ""));
        assert!(
            p == Plan::Drawdown {
                drawdown_value: "30".into(),
                drawdown_unit: Unit::Years,
                withdrawal: "2000".into(),
                strategy: Strategy::ProRata,
            }
        );
        // A blank withdrawal becomes "0" (a flat drawdown), never an empty string.
        let blank = plan_from(&drawing("  ", ""));
        assert!(matches!(
            blank,
            Plan::Drawdown { ref withdrawal, .. } if withdrawal == "0"
        ));
    }

    #[test]
    fn strategy_from_defaults_to_pro_rata() {
        // A link written before strategies existed, or against a build that had
        // different ones, must still project rather than refuse to open.
        for value in ["", "pro-rata", "nonsense"] {
            assert_eq!(strategy_from(value, ""), Strategy::ProRata, "{value}");
        }
        assert_eq!(strategy_from(STRATEGY_CHEAPEST, ""), Strategy::CheapestFirst);
        assert_eq!(strategy_from(STRATEGY_PRESERVE, ""), Strategy::PreserveGrowth);
        assert_eq!(
            strategy_from(STRATEGY_CAPPED, "20"),
            Strategy::RateCapped { max_rate: "20".into() }
        );
        // A blank cap is zero, not an empty string calc would reject.
        assert_eq!(
            strategy_from(STRATEGY_CAPPED, ""),
            Strategy::RateCapped { max_rate: "0".into() }
        );
    }

    #[test]
    fn the_conventional_order_comes_from_the_tax_system() {
        // Which accounts are spent first follows from how they are taxed, so it
        // is the tax system's judgement and must never be written out here.
        let expected: Vec<String> = TAX_SYSTEM
            .conventional_order()
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert_eq!(strategy_from(STRATEGY_CONVENTIONAL, ""), Strategy::Ordered { order: expected });
    }

    #[test]
    fn unknown_account_and_region_ids_fall_back_rather_than_erroring() {
        let first_kind = TAX_SYSTEM.account_kinds()[0].id;
        assert_eq!(kind_from("not_a_real_account"), first_kind);
        assert_eq!(kind_from(""), first_kind, "a pre-accounts link is untaxed");
        // A real id survives, whitespace and all.
        let real = TAX_SYSTEM.account_kinds().last().unwrap().id;
        assert_eq!(kind_from(&format!("  {real} ")), real);

        let first_region = TAX_SYSTEM.regions()[0].id;
        assert_eq!(region_from("narnia"), first_region);
        assert_eq!(region_from(""), first_region);
    }

    #[test]
    fn tax_details_are_withheld_from_a_pro_rata_projection() {
        // Pro-rata ignores tax entirely. Handing `calc` a context anyway would
        // make the output advertise a tax year it never used.
        assert!(tax_from(&drawing("2000", "")).is_none());
        assert!(tax_from(&drawing("2000", STRATEGY_PRO_RATA)).is_none());
        // And a deposits projection never has one, whatever the picker says.
        let deposits = FormInput { strategy: STRATEGY_CHEAPEST.into(), ..form(vec![], "10", "years") };
        assert!(tax_from(&deposits).is_none());

        let taxed = tax_from(&drawing("2000", STRATEGY_CHEAPEST)).expect("a tax-aware order needs one");
        assert_eq!(taxed.region, TAX_SYSTEM.regions()[0].id);
    }

    #[test]
    fn build_input_drops_fully_blank_rows() {
        let rows = vec![
            row(0, "A", "1000", "7", ""),
            row(1, "", "", "", ""), // fully blank -> dropped
            row(2, "C", "", "", "50"), // kept: has a contribution only
        ];
        let (input, ids) = build_input(&form(rows, "10", "years"));
        assert_eq!(input.investments.len(), 2);
        assert_eq!(ids, vec![0, 2]);
    }

    #[test]
    fn build_input_keeps_a_row_present_by_any_single_field() {
        for (v, r, c) in [("1", "", ""), ("", "5", ""), ("", "", "10")] {
            let (input, ids) = build_input(&form(vec![row(9, "", v, r, c)], "10", "years"));
            assert_eq!(input.investments.len(), 1, "value/rate/contrib = {v:?}/{r:?}/{c:?}");
            assert_eq!(ids, vec![9]);
        }
    }

    #[test]
    fn build_input_row_ids_survive_a_dropped_middle_row() {
        // The load-bearing case: dropping the middle row must not shift the
        // mapping.
        let rows = vec![
            row(11, "A", "1000", "", ""),
            row(12, "", "", "", ""), // dropped
            row(13, "C", "2000", "", ""),
        ];
        let (input, ids) = build_input(&form(rows, "10", "years"));
        assert_eq!(input.investments.len(), 2);
        assert_eq!(ids, vec![11, 13]);
    }

    #[test]
    fn build_input_defaults_blanks_and_names() {
        let (input, _) = build_input(&form(vec![row(0, "   ", "1000", "", "")], "  ", "years"));
        let inv = &input.investments[0];
        assert_eq!(inv.name, "Investment");
        assert_eq!(inv.rate, "0");
        assert_eq!(inv.contribution, "0");
        assert_eq!(inv.value, "1000");
        assert_eq!(input.horizon_value, "0");
    }

    #[test]
    fn build_input_threads_the_drawdown_plan_through() {
        let mut f = form(vec![row(0, "A", "1000", "7", "")], "10", "years");
        f.plan = "drawdown".into();
        f.withdrawal = "2000".into();
        let (input, _) = build_input(&f);
        assert!(
            input.plan == Plan::Drawdown {
                drawdown_value: "30".into(),
                drawdown_unit: Unit::Years,
                withdrawal: "2000".into(),
                strategy: Strategy::ProRata,
            }
        );
    }

    #[test]
    fn build_input_passes_horizon_through() {
        let (input, _) = build_input(&form(vec![row(0, "A", "1000", "7", "")], "36", "months"));
        assert_eq!(input.horizon_value, "36");
        assert!(input.horizon_unit == Unit::Months);
    }
}

//! The public vocabulary: projection inputs, outputs, and errors.
//!
//! Data types only — no arithmetic. [`Plan::Drawdown`] carries a
//! [`crate::strategy::Strategy`]; everything else is inert until [`crate::calculate`]
//! reads it.

use rust_decimal::Decimal;
use taxkit::{SimpleDate, TaxSystem};

use crate::strategy::Strategy;

/// The unit a period value is expressed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Years,
    Months,
}

/// What is being projected: a straight accumulation, or an accumulation followed
/// by a drawdown. Data-carrying so a deposits projection cannot hold a stale
/// drawdown period, and a drawdown cannot exist without both its period and its
/// withdrawal. The growth period itself is [`CalcInput::horizon_value`], shared
/// by both modes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Grow the portfolio over the horizon, adding each holding's monthly deposit.
    Deposits,
    /// Grow over the horizon, then draw the pot down over `drawdown_value` more
    /// time, taking `withdrawal` a month from the whole portfolio.
    Drawdown {
        drawdown_value: String,
        drawdown_unit: Unit,
        /// The portfolio-level monthly withdrawal, as a raw non-negative string
        /// (blank/`"0"` means a flat drawdown — just a longer accumulation).
        ///
        /// **Gross under [`Order::ProRata`], net under every other order.**
        /// Pro-rata predates the tax model and splits the money that *leaves the
        /// investments*; the ordered strategies exist to answer "how do I get
        /// £N into my pocket", which is a question about net.
        withdrawal: String,
        /// Which holdings the withdrawal comes out of, and in what order.
        strategy: Strategy,
    },
}

/// One investment as entered in the UI. Numbers arrive as strings (exactly as
/// typed) and are parsed here, so parsing and validation live in one place.
#[derive(Clone, Default)]
pub struct InvestmentInput {
    pub name: String,
    /// Today's value of the whole holding (principal plus any historical
    /// compounding already baked in). This is the figure projected forward.
    pub value: String,
    /// The annualised return, as a percent string (e.g. `"7"` for 7% a year).
    pub rate: String,
    /// The recurring monthly *deposit*, as a non-negative number (blank/`"0"`
    /// means none). Applied only during the accumulation phase.
    pub contribution: String,
    /// Which kind of account this holding sits in, as an **opaque id** from the
    /// active [`TaxSystem`]'s catalogue. Blank means the first kind advertised,
    /// which every system is expected to make its untaxed one — so a portfolio
    /// that says nothing about accounts projects exactly as it did before.
    /// Ignored entirely when there is no [`CalcInput::tax`].
    pub account_kind: String,
    /// What the holding originally cost, for account kinds taxed on the gain.
    /// **Blank means "today's value"** — no unrealised gain as things stand, and
    /// future growth is what becomes taxable. Ignored for other kinds.
    pub cost_basis: String,
}

/// Everything a tax system needs in order to price this portfolio's
/// withdrawals. `None` means an untaxed projection, which is what every
/// pre-existing input is.
///
/// `system` is a trait object precisely so `calc` never names a jurisdiction;
/// swapping it swaps the whole tax model.
#[derive(Clone)]
pub struct TaxContext {
    pub system: &'static dyn TaxSystem,
    /// An id from `system.regions()`.
    pub region: String,
    /// Other taxable income received each tax period, independent of the
    /// portfolio. Withdrawals are marginal on top of it.
    pub other_income: String,
    /// Age in whole years when the drawdown begins.
    pub age: String,
    /// Annual uprating of tax thresholds, as a percent string. Blank or `"0"`
    /// freezes them, which is the honest default under a policy of freezes but
    /// materially pessimistic over a thirty-year drawdown — hence the control.
    pub uprate: String,
    /// Bespoke per-jurisdiction inputs, as (id, value) string pairs, passed
    /// through to the tax system opaquely. Empty for a jurisdiction that asks
    /// for no extra controls. `calc` never interprets these.
    pub options: Vec<(String, String)>,
}

impl PartialEq for TaxContext {
    /// Systems are compared by name and rules period, not by pointer.
    ///
    /// A trait object's address is not a reliable identity — implementations are
    /// zero-sized, so distinct ones can share an address — and comparing vtable
    /// pointers is explicitly unreliable across codegen units. Name plus rules
    /// period is stable, cheap, and enough for the change detection this exists
    /// to serve.
    fn eq(&self, other: &Self) -> bool {
        self.system.label() == other.system.label()
            && self.system.rules_label() == other.system.rules_label()
            && self.region == other.region
            && self.other_income == other.other_income
            && self.age == other.age
            && self.uprate == other.uprate
            && self.options == other.options
    }
}

impl Eq for TaxContext {}

impl std::fmt::Debug for TaxContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaxContext")
            .field("system", &self.system.label())
            .field("rules", &self.system.rules_label())
            .field("region", &self.region)
            .field("other_income", &self.other_income)
            .field("age", &self.age)
            .field("uprate", &self.uprate)
            .field("options", &self.options)
            .finish()
    }
}

#[derive(Clone)]
pub struct CalcInput {
    pub investments: Vec<InvestmentInput>,
    /// The accumulation (growth) period. In drawdown mode this is the run-up
    /// before the withdrawals begin; the handover pot is measured at its end.
    pub horizon_value: String,
    pub horizon_unit: Unit,
    pub plan: Plan,
    /// Currency symbol for money embedded in error messages, supplied by the
    /// caller because `calc` names no currency of its own. Blank falls back to a
    /// neutral marker; when a [`TaxContext`] is present its system's symbol takes
    /// precedence, so a taxed and an untaxed projection over the same jurisdiction
    /// never disagree. Display-only — no arithmetic reads it.
    pub currency: String,
    /// The tax system to price withdrawals against. `None` is an untaxed
    /// projection.
    pub tax: Option<TaxContext>,
}

/// The currency marker used in money-bearing messages when no symbol is supplied
/// and no tax system offers one: the generic currency sign, deliberately not any
/// real money.
pub(crate) const NEUTRAL_CURRENCY: &str = "\u{00a4}";

impl CalcInput {
    /// Currency symbol for money embedded in error messages. `calc` names no
    /// currency, so this is the tax system's symbol when a context is present,
    /// otherwise the caller-supplied [`currency`](CalcInput::currency), and a
    /// neutral marker when neither is set. The app derives both from the same
    /// system, so they agree.
    pub(crate) fn currency_symbol(&self) -> &str {
        if let Some(t) = &self.tax {
            let s = t.system.currency_symbol();
            if !s.is_empty() {
                return s;
            }
        }
        if self.currency.is_empty() {
            NEUTRAL_CURRENCY
        } else {
            &self.currency
        }
    }
}

/// Which part of an investment row an error belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestmentField {
    Value,
    Rate,
    Contribution,
    /// The account-kind picker.
    AccountKind,
    /// The "what it originally cost" box, shown only for kinds taxed on gains.
    CostBasis,
}

/// The specific input a [`CalcError`] is about, so the UI can mark that control
/// invalid and describe it by the message rather than stranding a sentence at
/// the bottom of the form. `Investment::index` indexes
/// [`CalcInput::investments`] — the caller filters blank rows out before calling,
/// so it is the caller's job to map that back to its own row identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Investment { index: usize, part: InvestmentField },
    /// The growth period ("Project for" / "Grow for").
    Horizon,
    /// The drawdown period ("then draw down for").
    Drawdown,
    /// The portfolio-level monthly withdrawal.
    Withdrawal,
    /// Other taxable income per period.
    OtherIncome,
    /// Which part of the jurisdiction the holder lives in.
    Region,
    /// Age when the drawdown begins.
    Age,
    /// The withdrawal-order picker, and the rate cap that belongs to it.
    Strategy,
    /// Annual uprating of the tax thresholds.
    ///
    /// Its own variant rather than sharing [`Field::Strategy`]: that one marks
    /// the rate-cap box, which is only on screen under the rate-capped
    /// strategy, so an uprating error raised under any other strategy would
    /// mark no control at all and strand its message at the foot of the form.
    Uprate,
}

/// A validation or overflow failure. `field` is `None` when the problem is with
/// the portfolio as a whole rather than one control the user could go and fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalcError {
    pub message: String,
    pub field: Option<Field>,
}

impl CalcError {
    pub(crate) fn new(message: impl Into<String>, field: Option<Field>) -> Self {
        Self { message: message.into(), field }
    }

    pub(crate) fn at(message: impl Into<String>, index: usize, part: InvestmentField) -> Self {
        Self::new(message, Some(Field::Investment { index, part }))
    }
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Per-investment results. Monetary figures are rounded to 2 dp; `annualised`
/// is a growth *fraction* (e.g. 0.07 for 7%), left unrounded.
#[derive(Clone, Debug, PartialEq)]
pub struct InvestmentResult {
    pub name: String,
    pub current_value: Decimal,
    pub annualised: Decimal,
    /// Total this holding's monthly deposits add over the accumulation phase.
    /// Reported per row so `projected_value` reconciles: without it a holding
    /// with deposits looks like `current_value` grew at `annualised`, which it
    /// did not.
    pub contributed: Decimal,
    /// Total drawn from this holding over the drawdown phase (a positive figure),
    /// its pro-rata share of every month's portfolio withdrawal. Zero in
    /// deposits mode. Reported for the same reconciliation reason as `contributed`.
    pub withdrawn: Decimal,
    /// This holding's value at the start of drawdown — the pot the withdrawals
    /// draw from. `None` in deposits mode. Reported so a drawn-down row
    /// reconciles: the drawdown is measured against this, not `current_value`.
    pub handover_value: Option<Decimal>,
    pub projected_value: Decimal,
    /// Tax charged on this holding's withdrawals. Always zero without a
    /// [`TaxContext`], and always zero under [`Order::ProRata`].
    pub tax_paid: Decimal,
    /// `withdrawn - tax_paid`: what reached the holder from this holding.
    pub net_withdrawn: Decimal,
    /// Periodic tax charged on this holding for merely holding it over the
    /// projection (a wealth/accrual charge, not a withdrawal tax). Always zero
    /// unless the tax system sets `has_periodic_charge`. Reported for the same
    /// reconciliation reason as `contributed`: it reduces `projected_value`.
    pub charged: Decimal,
    /// The month this holding first hit £0, as an *absolute* index into
    /// `series`. Only meaningful once an ordered strategy can empty holdings at
    /// different times — under pro-rata they all empty together.
    pub depletion_month: Option<u32>,
    /// The account-kind id this holding was projected under, echoed back so the
    /// UI can resolve it to a label without re-reading the form.
    pub account_kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalcOutput {
    pub investments: Vec<InvestmentResult>,
    /// Portfolio total value at each month from 0 (today) to `total_months`.
    pub series: Vec<Decimal>,
    /// Cumulative deposits paid in by each month, parallel to `series`. Flat from
    /// `horizon_months` onward — deposits stop at the handover.
    pub contributions_series: Vec<Decimal>,
    /// Cumulative withdrawals taken by each month, parallel to `series`. Flat and
    /// zero through the accumulation phase; rises through the drawdown phase.
    pub withdrawals_series: Vec<Decimal>,
    /// The accumulation period, in months. Also the *index* of the handover point
    /// in every series.
    pub horizon_months: u32,
    /// The drawdown period, in months. `0` in deposits mode.
    pub drawdown_months: u32,
    /// `horizon_months + drawdown_months`, i.e. `series.len() - 1`.
    pub total_months: u32,
    pub current_total: Decimal,
    /// The pot at the start of drawdown (`series[horizon_months]`). `None` in
    /// deposits mode.
    pub handover_total: Option<Decimal>,
    /// Total of all monthly deposits added over the accumulation phase.
    pub contributed_total: Decimal,
    /// Total withdrawn across every holding over the drawdown phase (a positive
    /// figure). Zero in deposits mode.
    pub withdrawn_total: Decimal,
    /// The month the *whole portfolio* first reached £0, as an *absolute* index
    /// into `series`. `None` unless the combined total actually hits zero.
    ///
    /// Under [`Order::ProRata`] every holding empties in the same month, so
    /// this is also every row's depletion point; under an ordered strategy the
    /// rows empty in turn and this is the last of them. Either way it is the
    /// portfolio's single depletion point, which is what the goal-seek needs.
    pub depletion_month: Option<u32>,
    pub projected_total: Decimal,
    /// Projected investment gain: the final value less today's value *and* less
    /// the *net* cash you moved in along the way (deposits minus withdrawals), so
    /// it reflects returns only. Withdrawals — and any periodic charge — are
    /// added back: neither money you took out nor tax on holding is an
    /// investment loss.
    pub growth: Decimal,
    /// `growth` as a fraction of the capital deployed (today's value plus total
    /// deposits). A simple return on capital, not an IRR.
    pub growth_pct: Decimal,
    /// The capital `growth_pct` is measured against: `current_total +
    /// contributed_total`. Reported so the UI can state the basis instead of
    /// leaving a bare percentage the reader has to guess the denominator for.
    pub deployed: Decimal,

    // --- tax -----------------------------------------------------------------
    // `withdrawn_total` and `withdrawals_series` stay **gross**: they are
    // portfolio flows, the money that left the investments, and tax is a
    // downstream fact about that money. Keeping them gross is what leaves
    // `growth`, `deployed` and `growth_pct` byte-identical to the untaxed model
    // — netting them would book HMRC's slice as an investment loss, the exact
    // error `growth`'s definition exists to prevent. Tax is a *third* flow.
    /// Cumulative tax charged by each month, parallel to `series`. Zero through
    /// the accumulation phase: this model never taxes accumulation.
    ///
    /// A net-withdrawals series is deliberately *not* carried: it is exactly
    /// `withdrawals_series - tax_series` pointwise, so a caller that wants it
    /// zips the two rather than storing a third redundant vector.
    pub tax_series: Vec<Decimal>,
    pub tax_paid_total: Decimal,
    pub net_withdrawn_total: Decimal,
    /// `tax_paid_total / withdrawn_total`, zero when nothing was withdrawn.
    pub effective_tax_rate: Decimal,

    // --- periodic charge ----------------------------------------------------
    // A charge for *holding* rather than withdrawing (Germany's Vorabpauschale).
    // Zero for every withdrawal-only tax system, so these fields leave the UK
    // projection untouched. A charge reduces the pot without being a withdrawal
    // or a return, so it is its own flow — see the amended `growth` definition.
    /// Cumulative periodic charge by each month, parallel to `series`. May rise
    /// in *both* phases: you are taxed on the holding whether or not you draw.
    pub charged_series: Vec<Decimal>,
    /// Total periodic charge levied across the whole projection.
    pub charged_total: Decimal,
    /// Tax-free headroom that went unclaimed across the drawdown.
    ///
    /// The "show your working" figure: it is what explains *why* one withdrawal
    /// order beats another, and it turns a comparison of strategies from a
    /// scoreboard into an explanation.
    pub unused_allowance_total: Decimal,
    /// Months in the tax period `accounts_touched` is bucketed by — a fact
    /// owned by the tax system, so `None` on an untaxed
    /// projection, which has no fiscal period to count per. Reported rather than
    /// assumed to be twelve, which is a legislature-owned figure `calc` must not
    /// bake in.
    pub period_months: Option<u32>,
    /// How many distinct account kinds were drawn from in each tax period. The
    /// simplicity axis, and an honest counterweight to an optimiser that would
    /// have the holder touch four accounts every month. Without a tax system
    /// there are no periods, so the whole drawdown counts as a single one.
    pub accounts_touched: Vec<usize>,
    /// The typical number of account kinds touched in a period, rounded to
    /// nearest. `None` when there were no periods to average over.
    ///
    /// Reported rather than left for a caller to average, because *how* it
    /// rounds is a numeric policy: rounding up turns "one account in nine
    /// periods out of ten" into the same figure as "two every period", which is
    /// the one distinction the number exists to draw.
    pub accounts_touched_typical: Option<usize>,
    /// [`Limit::RateCap`] only: the cap had to be exceeded to deliver the
    /// requested income.
    pub rate_cap_breached: bool,
    /// The tax period the figures were computed under (e.g. "2026/27"), and when
    /// those figures were last checked. Copied off the [`TaxSystem`] so the UI
    /// never has to reach into a tax crate. `None` on an untaxed projection.
    pub rules_label: Option<&'static str>,
    pub rules_as_of: Option<SimpleDate>,
}

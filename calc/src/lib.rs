//! Investment projection core.
//!
//! Pure, exact base-10 `Decimal` arithmetic (via `rust_decimal`) — no UI, no
//! WASM bindings, no floating point. The Leptos front end calls [`calculate`]
//! directly with these types and only *formats* the `Decimal`s it gets back; it
//! performs no financial arithmetic of its own.
//!
//! A projection runs in one of two modes, carried by [`Plan`]:
//!
//! * **Deposits** — grow every holding from its value today, adding each row's
//!   optional monthly deposit, over the horizon. The classic accumulation.
//! * **Drawdown** — the same accumulation for `horizon_months`, then a second
//!   phase of `drawdown_months` in which a single *portfolio-level* monthly
//!   withdrawal is taken, apportioned across the holdings pro-rata by their
//!   current value and rebalanced every month. Monthly deposits stop at the
//!   handover; the only cash flow in the drawdown phase is that withdrawal.
//!
//! The whole thing is one continuous month-by-month series — `series[horizon_months]`
//! is the pot at the start of drawdown ([`CalcOutput::handover_total`]) — so the
//! UI never has to stitch two projections together or work out the handover value
//! for itself.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use rust_decimal::RoundingStrategy;
use std::str::FromStr;
use taxkit::{Draw, Pot, SessionSpec, SimpleDate, StopAt, TaxError, TaxErrorKind, TaxSession, TaxSystem};

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
        /// **Gross under [`Strategy::ProRata`], net under every other strategy.**
        /// Pro-rata predates the tax model and splits the money that *leaves the
        /// investments*; the ordered strategies exist to answer "how do I get
        /// £N into my pocket", which is a question about net.
        withdrawal: String,
        /// Which holdings the withdrawal comes out of, and in what order.
        strategy: Strategy,
    },
}

/// How a monthly withdrawal is apportioned across the holdings.
///
/// Every variant except [`Strategy::ProRata`] treats the withdrawal as a *net*
/// figure and needs to know what a draw costs, so all but [`Strategy::ProRata`]
/// and [`Strategy::PreserveGrowth`] require a [`TaxContext`].
///
/// Ordering is expressed over **opaque account-kind ids**, never over a named
/// enum of wrappers: `calc` carries the ids a [`TaxSystem`] advertises and never
/// learns what any of them mean.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Strategy {
    /// Split the withdrawal across every holding pro-rata by current value,
    /// rebalanced monthly, ignoring tax entirely. The original behaviour, and
    /// the default: an input that says nothing about tax projects exactly as it
    /// did before the tax model existed.
    #[default]
    ProRata,
    /// Empty each account kind in turn, in the order given, splitting pro-rata
    /// within a kind. Kinds present in the portfolio but missing from `order`
    /// are appended by their catalogue rank rather than treated as an error.
    Ordered { order: Vec<String> },
    /// Each month, take from whichever holding has the lowest marginal tax rate
    /// at that moment, re-checking at every rate boundary.
    ///
    /// Minimises tax *this month* exactly, and deliberately does not look
    /// further: it will happily drain a tax-free account early and leave a
    /// wholly taxable one to come out at the top rate. Lifetime-optimal is a
    /// dynamic program, not a greedy — hence the name.
    CheapestFirst,
    /// Drain the lowest-returning holding first, so the best compounder is left
    /// to compound. A **non-tax** objective: it needs no [`TaxContext`] at all
    /// and is legal on an untaxed projection.
    PreserveGrowth,
    /// Draw from a holding only while its marginal rate is at or below the cap
    /// (a percent string), then move on. If every holding is capped out and the
    /// requirement is still unmet, the shortfall is drawn anyway and
    /// [`CalcOutput::rate_cap_breached`] is set — delivering the money and
    /// saying so beats silently short-changing the withdrawal.
    RateCapped { max_rate: String },
}

impl Strategy {
    /// Whether this strategy needs to know what a withdrawal costs.
    fn needs_tax(&self) -> bool {
        !matches!(self, Strategy::ProRata | Strategy::PreserveGrowth)
    }

    /// Whether the plan's `withdrawal` is a net figure.
    pub fn withdrawal_is_net(&self) -> bool {
        !matches!(self, Strategy::ProRata)
    }
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
    /// The tax system to price withdrawals against. `None` is an untaxed
    /// projection.
    pub tax: Option<TaxContext>,
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
    fn new(message: impl Into<String>, field: Option<Field>) -> Self {
        Self { message: message.into(), field }
    }

    fn at(message: impl Into<String>, index: usize, part: InvestmentField) -> Self {
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
    /// [`TaxContext`], and always zero under [`Strategy::ProRata`].
    pub tax_paid: Decimal,
    /// `withdrawn - tax_paid`: what reached the holder from this holding.
    pub net_withdrawn: Decimal,
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
    /// Under [`Strategy::ProRata`] every holding empties in the same month, so
    /// this is also every row's depletion point; under an ordered strategy the
    /// rows empty in turn and this is the last of them. Either way it is the
    /// portfolio's single depletion point, which is what the goal-seek needs.
    pub depletion_month: Option<u32>,
    pub projected_total: Decimal,
    /// Projected investment gain: the final value less today's value *and* less
    /// the *net* cash you moved in along the way (deposits minus withdrawals), so
    /// it reflects returns only. Withdrawals are added back — money you took out
    /// is not an investment loss.
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
    pub tax_series: Vec<Decimal>,
    /// Cumulative net withdrawals — money in the holder's pocket — parallel to
    /// `series`. Derived from the *rounded* gross and tax series so the three
    /// reconcile pointwise on screen rather than only approximately.
    pub net_withdrawals_series: Vec<Decimal>,
    pub tax_paid_total: Decimal,
    pub net_withdrawn_total: Decimal,
    /// `tax_paid_total / withdrawn_total`, zero when nothing was withdrawn.
    pub effective_tax_rate: Decimal,
    /// Tax charged in each tax period of the drawdown, oldest first. The only
    /// way to see fiscal drag build up over time, and free — the ledger has it.
    pub tax_by_period: Vec<Decimal>,
    /// Tax-free headroom that went unclaimed across the drawdown.
    ///
    /// The "show your working" figure: it is what explains *why* one withdrawal
    /// order beats another, and it turns a comparison of strategies from a
    /// scoreboard into an explanation.
    pub unused_allowance_total: Decimal,
    /// How many distinct account kinds were drawn from in each tax period. The
    /// simplicity axis, and an honest counterweight to an optimiser that would
    /// have the holder touch four accounts every month.
    pub accounts_touched: Vec<usize>,
    /// The typical number of account kinds touched in a period, rounded to
    /// nearest. `None` when there were no periods to average over.
    ///
    /// Reported rather than left for a caller to average, because *how* it
    /// rounds is a numeric policy: rounding up turns "one account in nine
    /// periods out of ten" into the same figure as "two every period", which is
    /// the one distinction the number exists to draw.
    pub accounts_touched_typical: Option<usize>,    /// [`Strategy::RateCapped`] only: the cap had to be exceeded to deliver the
    /// requested income.
    pub rate_cap_breached: bool,
    /// The tax period the figures were computed under (e.g. "2026/27"), and when
    /// those figures were last checked. Copied off the [`TaxSystem`] so the UI
    /// never has to reach into a tax crate. `None` on an untaxed projection.
    pub rules_label: Option<&'static str>,
    pub rules_as_of: Option<SimpleDate>,
}

// --- ordered drawdown -------------------------------------------------------
//
// Everything below apportions a *net* monthly withdrawal across the holdings.
// None of it knows what a tax band is: it asks the session what a draw costs
// (`marginal_keep`) and when to stop (`Draw::rung_limited`), which is the whole
// reason a jurisdiction can be swapped without touching this file.

/// The greedy cannot loop for ever: each pass either meets the requirement,
/// empties a holding, blocks one, or crosses one rate boundary. This bounds the
/// last of those generously — the never-hang contract outranks the last penny.
const MAX_GREEDY_PASSES: usize = 24;

/// Mutable per-holding state one month of drawdown advances.
struct Ledger<'a> {
    balances: &'a mut [Decimal],
    /// What each holding originally cost, shrinking proportionally as it is
    /// sold. Only consulted for account kinds taxed on the gain.
    basis: &'a mut [Decimal],
    withdrawn: &'a mut [Decimal],
    taxes: &'a mut [Decimal],
}

/// Map a tax system's failure onto the control the user can go and fix.
///
/// The *message* is always the tax system's own, so `calc` never hard-codes an
/// age limit, an allowance or an account name; its job is only to pick a field.
fn tax_error(e: TaxError) -> CalcError {
    let field = match e.kind {
        TaxErrorKind::BadRegion => Some(Field::Region),
        TaxErrorKind::BadOtherIncome => Some(Field::OtherIncome),
        TaxErrorKind::AgeGated => Some(Field::Age),
        TaxErrorKind::Overflow | TaxErrorKind::BadRules | TaxErrorKind::UnknownAccount => None,
    };
    CalcError::new(e.message, field)
}

/// A blank numeric field reads as zero, matching the rest of the form.
fn parse_or_zero(s: &str) -> Option<Decimal> {
    if s.trim().is_empty() {
        Some(Decimal::ZERO)
    } else {
        parse_number(s)
    }
}

/// Open the tax session for a projection, validating the portfolio-level tax
/// inputs on the way.
fn open_session(tax: &TaxContext) -> Result<Box<dyn TaxSession>, CalcError> {
    let other_income = parse_or_zero(&tax.other_income).ok_or_else(|| {
        CalcError::new("Enter a valid amount of other taxable income.", Some(Field::OtherIncome))
    })?;
    if other_income < Decimal::ZERO {
        return Err(CalcError::new(
            "Other taxable income cannot be negative.",
            Some(Field::OtherIncome),
        ));
    }

    let age = if tax.age.trim().is_empty() {
        None
    } else {
        let a = parse_number(&tax.age)
            .ok_or_else(|| CalcError::new("Enter a valid age.", Some(Field::Age)))?;
        if a < Decimal::ZERO || a > Decimal::from(120u32) {
            return Err(CalcError::new("Enter an age between 0 and 120.", Some(Field::Age)));
        }
        a.to_u32()
    };

    let uprate = parse_or_zero(&tax.uprate)
        .ok_or_else(|| CalcError::new("Enter a valid uprating percentage.", Some(Field::Uprate)))?
        / Decimal::from(100u32);
    if uprate <= Decimal::NEGATIVE_ONE {
        return Err(CalcError::new(
            "Tax thresholds cannot shrink by 100% or more a year.",
            Some(Field::Uprate),
        ));
    }

    tax.system
        .open(&SessionSpec { region: tax.region.clone(), other_income, age, uprate })
        .map_err(tax_error)
}

/// How the greedy ranks one holding against another, cheapest first.
///
/// `keep` is what survives tax on the next pound out. `expiring` breaks ties
/// between equally cheap holdings, and is the difference between an optimiser
/// and a fixed order: an account that is cheap because an allowance has not been
/// spent is use-it-or-lose-it, so it must be drawn *before* one that is cheap
/// indefinitely. Rank and input order then settle the rest, so the answer never
/// depends on sort stability.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Priority {
    keep: Decimal,
    expiring: bool,
    rank: u8,
    index: usize,
}

impl Priority {
    fn beats(&self, other: &Priority) -> bool {
        (self.keep, self.expiring, std::cmp::Reverse(self.rank), std::cmp::Reverse(self.index))
            > (other.keep, other.expiring, std::cmp::Reverse(other.rank), std::cmp::Reverse(other.index))
    }
}

fn pot_of(
    kinds: &[Option<&'static taxkit::AccountKind>],
    led: &Ledger,
    j: usize,
) -> Option<Pot> {
    kinds[j].map(|k| Pot {
        kind: k.id,
        available: led.balances[j],
        cost_basis: led.basis[j],
    })
}

fn priority_of(
    session: &Option<Box<dyn TaxSession>>,
    kinds: &[Option<&'static taxkit::AccountKind>],
    led: &Ledger,
    j: usize,
) -> Priority {
    let (keep, expiring) = match (session, pot_of(kinds, led, j)) {
        (Some(s), Some(pot)) => (
            s.marginal_keep(&pot),
            // Finite headroom at this rate means the cheapness runs out.
            s.marginal_headroom(&pot).is_some(),
        ),
        _ => (Decimal::ONE, false),
    };
    Priority { keep, expiring, rank: kinds[j].map_or(0, |k| k.rank), index: j }
}

/// Take up to `net_wanted` out of one holding and post it to the ledger.
fn draw_one(
    session: &mut Option<Box<dyn TaxSession>>,
    kinds: &[Option<&'static taxkit::AccountKind>],
    led: &mut Ledger,
    j: usize,
    net_wanted: Decimal,
    stop: StopAt,
) -> Result<Draw, CalcError> {
    let available = led.balances[j];
    if available <= Decimal::ZERO || net_wanted <= Decimal::ZERO {
        return Ok(Draw::default());
    }

    let drawn = match (session.as_mut(), kinds[j]) {
        (Some(s), Some(k)) => {
            let pot = Pot { kind: k.id, available, cost_basis: led.basis[j] };
            s.draw(&pot, net_wanted, stop).map_err(tax_error)?
        }
        // No tax system: every account is free, so gross is net.
        _ => {
            let g = net_wanted.min(available);
            Draw { gross: g, tax: Decimal::ZERO, net: g, rung_limited: false }
        }
    };

    if drawn.gross > Decimal::ZERO {
        // Proportional disposal: a slice carries the same fraction of the
        // original cost away with it, so what remains keeps its profit ratio.
        // That also makes the taxable fraction path-independent, which is what
        // keeps the goal-seek's feasibility search well behaved.
        let left = (available - drawn.gross).max(Decimal::ZERO);
        led.basis[j] = led.basis[j]
            .checked_mul(left)
            .and_then(|x| x.checked_div(available))
            .ok_or_else(overflowed)?;
        led.balances[j] = left;
        led.withdrawn[j] += drawn.gross;
        led.taxes[j] += drawn.tax;
    }
    Ok(drawn)
}

/// One month of a **static** order: work through the groups in turn, splitting
/// each group's share pro-rata by balance with the residue on its last member,
/// then mopping up anything the split could not deliver.
///
/// The residue rule is the existing pro-rata invariant, generalised: every
/// apportionment's rounding lands on the last member of the group it
/// apportions, so per-row withdrawals still sum exactly to the total.
fn static_month(
    session: &mut Option<Box<dyn TaxSession>>,
    kinds: &[Option<&'static taxkit::AccountKind>],
    led: &mut Ledger,
    groups: &[Vec<usize>],
    net_wanted: Decimal,
) -> Result<Decimal, CalcError> {
    let mut delivered = Decimal::ZERO;
    for group in groups {
        let remaining = net_wanted - delivered;
        if remaining <= Decimal::ZERO {
            break;
        }
        let mut group_bal = Decimal::ZERO;
        for j in group {
            group_bal = group_bal
                .checked_add(led.balances[*j])
                .ok_or_else(overflowed)?;
        }
        if group_bal <= Decimal::ZERO {
            continue;
        }

        let mut allocated = Decimal::ZERO;
        for (k, j) in group.iter().enumerate() {
            let share = if k + 1 == group.len() {
                remaining - allocated
            } else {
                remaining
                    .checked_mul(led.balances[*j])
                    .and_then(|x| x.checked_div(group_bal))
                    .ok_or_else(overflowed)?
            };
            allocated += share;
            delivered += draw_one(session, kinds, led, *j, share, StopAt::Requirement)?.net;
        }

        // A member may not have managed its share -- its balance ran out, or tax
        // took more of it than a split by balance assumed. Sweep the group once
        // before moving on, so the shortfall does not silently leak away.
        for j in group {
            let short = net_wanted - delivered;
            if short <= Decimal::ZERO {
                break;
            }
            delivered += draw_one(session, kinds, led, *j, short, StopAt::Requirement)?.net;
        }
    }
    Ok(delivered)
}

/// One month of a **dynamic** order: repeatedly take from whichever holding
/// keeps most of the next pound, stopping at each rate boundary so the choice is
/// reconsidered.
///
/// This is what makes it an optimiser rather than a fixed order — it will fill a
/// zero-rate allowance from a taxable account, switch to a tax-free one, and
/// switch back when that runs out — and it does so without knowing that any of
/// those things exist.
///
/// The holding to draw from next: cheapest first, by [`Priority`].
///
/// `blocked` is empty on the cap-breach pass, which reconsiders everything.
/// Extracted because the tie-breaking in `Priority::beats` is the subtle part of
/// the optimiser, and it had two call sites that had to stay in step by hand.
fn best_holding(
    session: &Option<Box<dyn TaxSession>>,
    kinds: &[Option<&'static taxkit::AccountKind>],
    led: &Ledger,
    blocked: &[bool],
) -> Option<usize> {
    let mut best: Option<Priority> = None;
    for j in 0..led.balances.len() {
        if blocked.get(j).copied().unwrap_or(false) || led.balances[j] <= Decimal::ZERO {
            continue;
        }
        let p = priority_of(session, kinds, led, j);
        if best.is_none_or(|b| p.beats(&b)) {
            best = Some(p);
        }
    }
    best.map(|p| p.index)
}

/// Returns the net delivered and whether a rate cap had to be breached.
fn greedy_month(
    session: &mut Option<Box<dyn TaxSession>>,
    kinds: &[Option<&'static taxkit::AccountKind>],
    led: &mut Ledger,
    net_wanted: Decimal,
    cap: Option<Decimal>,
    // Scratch, owned by the caller so the month loop reuses one allocation.
    blocked: &mut [bool],
) -> Result<(Decimal, bool), CalcError> {
    let n = led.balances.len();
    let stop = match cap {
        Some(c) => StopAt::RateAbove(c),
        None => StopAt::NextRung,
    };
    let mut delivered = Decimal::ZERO;
    blocked.fill(false);

    for _ in 0..(n * MAX_GREEDY_PASSES + 1) {
        let remaining = net_wanted - delivered;
        if remaining <= Decimal::ZERO {
            break;
        }
        let Some(j) = best_holding(session, kinds, led, blocked) else { break };

        let drawn = draw_one(session, kinds, led, j, remaining, stop)?;
        delivered += drawn.net;
        if drawn.gross <= Decimal::ZERO {
            // It gave nothing: either the cap shut it out or it is empty. Either
            // way it cannot help under this stop, so drop it from contention.
            blocked[j] = true;
        }
    }

    // A cap that cannot be honoured. Delivering the money and reporting the
    // breach beats silently handing over less than was asked for.
    let mut breached = false;
    if cap.is_some() {
        for _ in 0..(n + 1) {
            let short = net_wanted - delivered;
            if short <= Decimal::ZERO {
                break;
            }
            let Some(j) = best_holding(session, kinds, led, &[]) else { break };
            let drawn = draw_one(session, kinds, led, j, short, StopAt::Requirement)?;
            if drawn.gross <= Decimal::ZERO {
                break;
            }
            // An untaxed holding is never shut out by a rate cap, so anything
            // this pass manages to draw was drawn above the cap.
            breached = true;
            delivered += drawn.net;
        }
    }

    Ok((delivered, breached))
}

/// Group holdings by account kind, in the order the caller asked for.
///
/// Kinds present in the portfolio but missing from `order` are appended by
/// catalogue rank rather than rejected: a forgiving rule that adds no new way
/// for the projection to fail.
fn groups_by_kind(
    order: &[String],
    kinds: &[Option<&'static taxkit::AccountKind>],
) -> Vec<Vec<usize>> {
    let id_of = |j: usize| kinds[j].map_or("", |k| k.id);

    let mut present: Vec<(&'static str, u8)> = Vec::new();
    for (j, k) in kinds.iter().enumerate() {
        let id = id_of(j);
        if !present.iter().any(|(p, _)| *p == id) {
            present.push((id, k.map_or(0, |k| k.rank)));
        }
    }

    let mut sequence: Vec<&'static str> = Vec::with_capacity(present.len());
    for want in order {
        if let Some((id, _)) = present.iter().find(|(p, _)| *p == want.as_str()) {
            if !sequence.contains(id) {
                sequence.push(id);
            }
        }
    }
    let mut rest: Vec<(&'static str, u8)> = present
        .iter()
        .filter(|(p, _)| !sequence.contains(p))
        .copied()
        .collect();
    rest.sort_by_key(|(id, rank)| (*rank, *id));
    sequence.extend(rest.into_iter().map(|(id, _)| id));

    sequence
        .into_iter()
        .map(|id| (0..kinds.len()).filter(|j| id_of(*j) == id).collect())
        .collect()
}

/// Group holdings by annualised return, lowest first: drain the worst
/// compounder before touching the best. Equal-returning holdings share a group
/// and are drawn pro-rata, so the order never depends on how rows were typed.
fn groups_by_return(annuals: &[Decimal]) -> Vec<Vec<usize>> {
    let mut distinct: Vec<Decimal> = annuals.to_vec();
    distinct.sort();
    distinct.dedup();
    distinct
        .into_iter()
        .map(|r| (0..annuals.len()).filter(|j| annuals[*j] == r).collect())
        .collect()
}

/// Project a portfolio forward. Returns a user-facing message on any invalid
/// input rather than panicking.
pub fn calculate(input: &CalcInput) -> Result<CalcOutput, CalcError> {
    if input.investments.is_empty() {
        return Err(CalcError::new("Add at least one investment.", None));
    }

    let hundred = Decimal::from(100u32);
    let twelve = Decimal::from(12u32);

    let horizon_months = to_months(&input.horizon_value, input.horizon_unit, "The growth period")
        .map_err(|m| CalcError::new(m, Some(Field::Horizon)))?;
    if horizon_months < 1 {
        return Err(CalcError::new(
            "Enter a growth period of at least 1 month.",
            Some(Field::Horizon),
        ));
    }
    if horizon_months > MAX_HORIZON_MONTHS {
        return Err(CalcError::new(
            "The growth period is limited to 100 years (1200 months).",
            Some(Field::Horizon),
        ));
    }

    // The drawdown phase, if any: its length, the portfolio withdrawal taken
    // each month, and how that withdrawal is apportioned. Deposits mode has none.
    let default_strategy = Strategy::ProRata;
    let strategy = match &input.plan {
        Plan::Deposits => &default_strategy,
        Plan::Drawdown { strategy, .. } => strategy,
    };
    let (drawdown_months, withdrawal) = match &input.plan {
        Plan::Deposits => (0u32, Decimal::ZERO),
        Plan::Drawdown { drawdown_value, drawdown_unit, withdrawal, .. } => {
            let d = to_months(drawdown_value, *drawdown_unit, "The drawdown period")
                .map_err(|m| CalcError::new(m, Some(Field::Drawdown)))?;
            if d < 1 {
                return Err(CalcError::new(
                    "Enter a drawdown period of at least 1 month.",
                    Some(Field::Drawdown),
                ));
            }
            if horizon_months as u64 + d as u64 > MAX_HORIZON_MONTHS as u64 {
                return Err(CalcError::new(
                    "The growth and drawdown periods together are limited to 100 years (1200 months).",
                    Some(Field::Drawdown),
                ));
            }
            let w = parse_number(withdrawal)
                .ok_or_else(|| CalcError::new("Enter a valid monthly withdrawal.", Some(Field::Withdrawal)))?;
            if w < Decimal::ZERO {
                return Err(CalcError::new(
                    "The monthly withdrawal cannot be negative.",
                    Some(Field::Withdrawal),
                ));
            }
            (d, w)
        }
    };

    // The tax system, if this projection has one. The catalogue is needed even
    // in deposits mode, because account kinds are validated per row regardless.
    let catalogue: &'static [taxkit::AccountKind] =
        input.tax.as_ref().map_or(&[], |t| t.system.account_kinds());

    // A rate cap belongs to the strategy that carries it, so its errors point at
    // the strategy control rather than stranding a sentence at the foot of the
    // form.
    let rate_cap = match strategy {
        Strategy::RateCapped { max_rate } => {
            let r = parse_or_zero(max_rate).ok_or_else(|| {
                CalcError::new("Enter a valid rate to cap withdrawals at.", Some(Field::Strategy))
            })? / Decimal::from(100u32);
            if r < Decimal::ZERO {
                return Err(CalcError::new(
                    "The rate cap cannot be negative.",
                    Some(Field::Strategy),
                ));
            }
            Some(r)
        }
        _ => None,
    };

    let total_months = horizon_months + drawdown_months;
    let horizon = horizon_months as usize;
    let total = total_months as usize;
    let drawing = drawdown_months > 0;

    // Parse and validate every holding up front, deriving its monthly growth
    // factor. The month loop below is *month-major* (all holdings advance one
    // month together) because the drawdown split depends on every holding's
    // current balance at once, so per-holding state can't run in isolation.
    struct Prepared {
        name: String,
        current_value: Decimal,
        contribution: Decimal,
        monthly: Decimal,
        annual: Decimal,
        /// The account kind this holding sits in, resolved against the active
        /// tax system's catalogue. `None` on an untaxed projection, where the
        /// notion does not apply.
        kind: Option<&'static taxkit::AccountKind>,
        /// The id as given, echoed back on the result so the UI can label the
        /// row without re-reading the form. Survives having no catalogue.
        kind_id: String,
        cost_basis: Decimal,
    }
    let mut prepared: Vec<Prepared> = Vec::with_capacity(input.investments.len());

    for (index, inv) in input.investments.iter().enumerate() {
        use InvestmentField::{Contribution, Rate, Value};
        let too_large = |part| CalcError::at(too_large_msg(&inv.name), index, part);

        let current_value = parse_number(&inv.value)
            .ok_or_else(|| CalcError::at(format!("'{}' has an invalid amount.", inv.name), index, Value))?;
        if current_value < Decimal::ZERO {
            return Err(CalcError::at(format!("'{}' has a negative amount.", inv.name), index, Value));
        }

        let contribution = parse_number(&inv.contribution).ok_or_else(|| {
            CalcError::at(format!("'{}' has an invalid monthly deposit.", inv.name), index, Contribution)
        })?;
        if contribution < Decimal::ZERO {
            return Err(CalcError::at(
                format!("'{}' has a negative monthly amount.", inv.name),
                index,
                Contribution,
            ));
        }

        let rate = parse_number(&inv.rate)
            .ok_or_else(|| CalcError::at(format!("'{}' has an invalid rate.", inv.name), index, Rate))?
            / hundred;
        // 1 + rate must stay positive for real-valued compounding / roots.
        if rate <= Decimal::NEGATIVE_ONE {
            return Err(CalcError::at(
                format!("'{}' has a return of -100% or worse, which cannot be projected.", inv.name),
                index,
                Rate,
            ));
        }
        // The annualised input drives the projection directly; the monthly factor
        // is its geometric twelfth root, one `powd` per holding. Checked, because
        // an extreme-but-reachable input (100% annualised over 100 years) exceeds
        // the Decimal maximum and an unchecked `powd` would panic.
        let annual = rate;
        let monthly = (Decimal::ONE + annual)
            .checked_powd(Decimal::ONE / twelve)
            .ok_or_else(|| too_large(Rate))?;

        // Which account this holding sits in. Blank picks the first kind the
        // system advertises — every system is expected to lead with its untaxed
        // one, so a portfolio that says nothing about accounts keeps behaving as
        // it always did.
        let wanted = inv.account_kind.trim();
        let kind = if catalogue.is_empty() {
            None
        } else if wanted.is_empty() {
            catalogue.first()
        } else {
            Some(catalogue.iter().find(|k| k.id == wanted).ok_or_else(|| {
                CalcError::at(
                    format!("'{}' is in an account this calculator does not know.", inv.name),
                    index,
                    InvestmentField::AccountKind,
                )
            })?)
        };

        // What it originally cost, for kinds taxed on the gain. Blank means
        // "today's value" — nothing gained yet, and future growth is what
        // becomes taxable. Only validated where it is actually consulted, so a
        // leftover figure on a wrapper that ignores it is not an error.
        let cost_basis = if kind.is_some_and(|k| k.needs_cost_basis) && !inv.cost_basis.trim().is_empty() {
            let c = parse_number(&inv.cost_basis).ok_or_else(|| {
                CalcError::at(
                    format!("'{}' has an invalid original cost.", inv.name),
                    index,
                    InvestmentField::CostBasis,
                )
            })?;
            if c < Decimal::ZERO {
                return Err(CalcError::at(
                    format!("'{}' has a negative original cost.", inv.name),
                    index,
                    InvestmentField::CostBasis,
                ));
            }
            c
        } else {
            current_value
        };

        prepared.push(Prepared {
            name: inv.name.clone(),
            current_value,
            contribution,
            monthly,
            annual,
            kind,
            kind_id: kind.map_or_else(|| wanted.to_string(), |k| k.id.to_string()),
            cost_basis,
        });
    }

    let n = prepared.len();
    // Per-holding running balance and cumulative cash flow.
    let mut balances: Vec<Decimal> = prepared.iter().map(|p| p.current_value).collect();
    let mut basis: Vec<Decimal> = prepared.iter().map(|p| p.cost_basis).collect();
    let mut contributed: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut withdrawn: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut taxes: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut handover: Vec<Option<Decimal>> = vec![None; n];
    let mut row_depletion: Vec<Option<u32>> = vec![None; n];
    // Whether a holding has ever actually held anything. Tracked rather than
    // read off `current_value`, because a holding can be built up entirely from
    // deposits: it starts at zero, and its value today says nothing about
    // whether it later had something to run out of.
    let mut ever_held: Vec<bool> = vec![false; n];

    let kinds: Vec<Option<&'static taxkit::AccountKind>> =
        prepared.iter().map(|p| p.kind).collect();
    let annuals: Vec<Decimal> = prepared.iter().map(|p| p.annual).collect();

    // An ordered strategy needs to know what a draw costs. Pro-rata never opens
    // a session at all, which is what keeps it byte-identical to the untaxed
    // model; `PreserveGrowth` will use one if there is one but does not need it,
    // because it orders by return rather than by tax.
    let ordered = drawing && *strategy != Strategy::ProRata;
    let mut session: Option<Box<dyn TaxSession>> = None;
    if ordered {
        match &input.tax {
            Some(tax) => session = Some(open_session(tax)?),
            None if strategy.needs_tax() => {
                return Err(CalcError::new(
                    "This withdrawal order needs to know how the accounts are taxed. \
                     Fill in the tax details, or split the withdrawal pro-rata instead.",
                    Some(Field::Strategy),
                ))
            }
            None => {}
        }
    }

    // Static orders are the same every month, so they are worked out once.
    let groups: Vec<Vec<usize>> = match strategy {
        Strategy::Ordered { order } => groups_by_kind(order, &kinds),
        Strategy::PreserveGrowth => groups_by_return(&annuals),
        _ => Vec::new(),
    };

    let period_months = session.as_ref().map_or(12, |s| s.period_months()).max(1);
    // Scratch reused by every drawdown month rather than reallocated per month:
    // `before` snapshots the balances to diff against, `blocked` is the greedy's
    // out-of-contention set. Both are overwritten in full before each use.
    let mut before: Vec<Decimal> = vec![Decimal::ZERO; n];
    let mut blocked: Vec<bool> = vec![false; n];
    let mut tax_by_period: Vec<Decimal> = Vec::new();
    let mut accounts_touched: Vec<usize> = Vec::new();
    let mut period_kinds: Vec<&str> = Vec::new();
    let mut rate_cap_breached = false;

    // Portfolio series, one point per month inclusive of both endpoints.
    let mut totals: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut contribs: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut withdraws: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut taxed: Vec<Decimal> = vec![Decimal::ZERO; total + 1];
    let mut contributed_total = Decimal::ZERO;
    let mut withdrawn_total = Decimal::ZERO;
    let mut tax_total = Decimal::ZERO;

    // Rate-attributed and contribution-attributed overflow helpers for the loop.
    let grow_too_large = |index: usize| CalcError::at(too_large_msg(&prepared[index].name), index, InvestmentField::Rate);
    let dep_too_large = |index: usize| CalcError::at(too_large_msg(&prepared[index].name), index, InvestmentField::Contribution);

    for i in 0..=total {
        // Record the start-of-month portfolio state before any flow this month.
        // Only the grown balances need re-summing; the cumulative cash flows are
        // already tracked exactly as running scalars (`Σ contributed[j]` and
        // `Σ withdrawn[j]` equal them by construction — see the residue rule below).
        let mut tsum = Decimal::ZERO;
        for j in 0..n {
            tsum = tsum.checked_add(balances[j]).ok_or_else(overflowed)?;
        }
        totals[i] = tsum;
        contribs[i] = contributed_total;
        withdraws[i] = withdrawn_total;
        taxed[i] = tax_total;

        // A holding is spent the first month it reaches zero having had
        // something in it. Under pro-rata they all land together; under an
        // ordered strategy they empty in turn, and this is the only way to see
        // which one is carrying the drawdown at any point.
        for j in 0..n {
            if balances[j] > Decimal::ZERO {
                ever_held[j] = true;
            } else if row_depletion[j].is_none() && ever_held[j] && balances[j].is_zero() {
                row_depletion[j] = Some(i as u32);
            }
        }

        // The handover pot is the start-of-month value at the accumulation
        // boundary, before the first withdrawal.
        if drawing && i == horizon {
            for j in 0..n {
                handover[j] = Some(balances[j]);
            }
        }

        if i == total {
            break; // endpoint: record only, apply no flow.
        }

        // Grow every holding one month.
        for j in 0..n {
            balances[j] = balances[j].checked_mul(prepared[j].monthly).ok_or_else(|| grow_too_large(j))?;
        }

        if i < horizon {
            // Accumulation: add each holding's monthly deposit.
            for j in 0..n {
                let c = prepared[j].contribution;
                balances[j] = balances[j].checked_add(c).ok_or_else(|| dep_too_large(j))?;
                contributed[j] = contributed[j].checked_add(c).ok_or_else(|| dep_too_large(j))?;
                contributed_total = contributed_total.checked_add(c).ok_or_else(|| dep_too_large(j))?;
            }
        } else if ordered {
            // An ordered drawdown. `withdrawal` is the NET the holder wants in
            // their pocket, so the amount leaving the investments is whatever it
            // takes to deliver that after tax.
            //
            // Tax periods are anchored to the *handover*, not to a real calendar
            // date and not to month zero. A real date would make the projection
            // depend on when it was run, which would break shared links and make
            // the browser suite fail every April; either alternative would also
            // manufacture a stub first period carrying a full year's allowances,
            // since allowances are not pro-rated for part periods.
            let month_of_drawdown = i - horizon;
            // Month zero opens the first period rather than closing one, so the
            // boundary only bites from the second period on.
            if month_of_drawdown > 0 && month_of_drawdown % (period_months as usize) == 0 {
                if let Some(s) = session.as_mut() {
                    tax_by_period.push(round2(s.period_tax()));
                    s.start_period();
                }
                accounts_touched.push(period_kinds.len());
                period_kinds.clear();
            }

            before.copy_from_slice(&balances);
            let mut led = Ledger {
                balances: &mut balances,
                basis: &mut basis,
                withdrawn: &mut withdrawn,
                taxes: &mut taxes,
            };
            match strategy {
                Strategy::Ordered { .. } | Strategy::PreserveGrowth => {
                    static_month(&mut session, &kinds, &mut led, &groups, withdrawal)?;
                }
                _ => {
                    let (_, breached) = greedy_month(
                        &mut session,
                        &kinds,
                        &mut led,
                        withdrawal,
                        rate_cap,
                        &mut blocked,
                    )?;
                    rate_cap_breached |= breached;
                }
            }

            // Roll the month's per-holding movement up into the portfolio totals.
            for j in 0..n {
                let took = before[j] - balances[j];
                if took > Decimal::ZERO {
                    withdrawn_total = withdrawn_total
                        .checked_add(took)
                        .ok_or_else(overflowed)?;
                    let id = prepared[j].kind_id.as_str();
                    if !period_kinds.contains(&id) {
                        period_kinds.push(id);
                    }
                }
            }
            tax_total = taxes.iter().sum();
        } else {
            // Drawdown: take the portfolio withdrawal, apportioned across the
            // holdings pro-rata by their current balance. Capped at the whole
            // pot — an empty portfolio yields nothing further.
            let mut total_bal = Decimal::ZERO;
            for j in 0..n {
                total_bal = total_bal.checked_add(balances[j]).ok_or_else(overflowed)?;
            }
            if !total_bal.is_zero() {
                let drawn = withdrawal.min(total_bal).max(Decimal::ZERO);
                if drawn == total_bal {
                    // Draws the lot: zero every balance *exactly* rather than by
                    // subtraction, so no sub-penny residue lingers and depletion
                    // reads cleanly.
                    for j in 0..n {
                        withdrawn[j] = withdrawn[j].checked_add(balances[j]).ok_or_else(overflowed)?;
                        balances[j] = Decimal::ZERO;
                    }
                } else {
                    // Pro-rata shares. The last holding absorbs the rounding
                    // residue so the shares sum to `drawn` by construction — this
                    // is what keeps `Σ per-row withdrawn == withdrawn_total` exact.
                    let mut allocated = Decimal::ZERO;
                    for j in 0..n - 1 {
                        let share = drawn
                            .checked_mul(balances[j])
                            .and_then(|x| x.checked_div(total_bal))
                            .ok_or_else(overflowed)?;
                        balances[j] -= share;
                        withdrawn[j] += share;
                        allocated += share;
                    }
                    let last = drawn - allocated;
                    balances[n - 1] -= last;
                    withdrawn[n - 1] += last;
                }
                withdrawn_total = withdrawn_total.checked_add(drawn).ok_or_else(overflowed)?;
            }
        }
    }

    let series: Vec<Decimal> = totals.iter().map(|v| round2(*v)).collect();
    let contributions_series: Vec<Decimal> = contribs.iter().map(|v| round2(*v)).collect();
    let withdrawals_series: Vec<Decimal> = withdraws.iter().map(|v| round2(*v)).collect();
    let current_total = round2(*totals.first().expect("horizon >= 1 guarantees a point"));
    let projected_total = round2(*totals.last().expect("horizon >= 1 guarantees a point"));
    let handover_total = if drawing { Some(round2(totals[horizon])) } else { None };
    let contributed_total = round2(contributed_total);
    let withdrawn_total = round2(withdrawn_total);

    // The whole portfolio "runs out" only when its combined total actually hits
    // zero *having held something first*. Scan the **unrounded** totals from the
    // first month there was anything to spend, rather than gating on month zero:
    // a portfolio built entirely from deposits starts at nothing and would
    // otherwise never report running out. A portfolio that is never worth
    // anything still reports `None`, which is the degenerate case worth skipping.
    let depletion_month = totals
        .iter()
        .position(|v| *v > Decimal::ZERO)
        .and_then(|start| {
            totals[start..]
                .iter()
                .position(|v| v.is_zero())
                .map(|k| (start + k) as u32)
        });

    // The last tax period never reaches a boundary, so close it here.
    if ordered {
        if let Some(s) = session.as_ref() {
            tax_by_period.push(round2(s.period_tax()));
        }
        accounts_touched.push(period_kinds.len());
    }
    // Only reported when a session actually priced something. A pro-rata run
    // ignores tax entirely, so advertising which tax year it used would imply a
    // calculation it never did -- and it is what keeps pro-rata byte-identical
    // to the untaxed model.
    let (rules_label, rules_as_of) = match (&session, &input.tax) {
        (Some(_), Some(t)) => (Some(t.system.rules_label()), Some(t.system.as_of())),
        _ => (None, None),
    };
    // Round-half-up in integers: no float enters a reported figure.
    let accounts_touched_typical = (!accounts_touched.is_empty()).then(|| {
        let total: usize = accounts_touched.iter().sum();
        let periods = accounts_touched.len();
        (total * 2 + periods) / (periods * 2)
    });
    let unused_allowance_total = session
        .as_ref()
        .map_or(Decimal::ZERO, |s| round2(s.unused_allowance()));

    let results: Vec<InvestmentResult> = prepared
        .iter()
        .enumerate()
        .map(|(j, p)| {
            // Round gross and tax independently, then derive net from the
            // rounded pair, so the three reconcile exactly on screen rather than
            // to within a penny.
            let gross = round2(withdrawn[j]);
            let tax = round2(taxes[j]);
            InvestmentResult {
                name: p.name.clone(),
                current_value: round2(p.current_value),
                annualised: p.annual,
                contributed: round2(contributed[j]),
                withdrawn: gross,
                handover_value: handover[j].map(round2),
                projected_value: round2(balances[j]),
                tax_paid: tax,
                net_withdrawn: gross - tax,
                depletion_month: row_depletion[j],
                account_kind: p.kind_id.clone(),
            }
        })
        .collect();

    // Gain from returns only: strip out today's value and the *net* cash moved in
    // (deposits minus withdrawals), so money withdrawn is not booked as a loss.
    // Percentage is against the capital deployed (today's value plus deposits).
    let net_contributed = contributed_total
        .checked_sub(withdrawn_total)
        .ok_or_else(overflowed)?;
    let growth = projected_total
        .checked_sub(current_total)
        .and_then(|g| g.checked_sub(net_contributed))
        .ok_or_else(overflowed)?;
    let deployed = current_total
        .checked_add(contributed_total)
        .ok_or_else(overflowed)?;
    let growth_pct = growth.checked_div(deployed).unwrap_or(Decimal::ZERO);

    let tax_series: Vec<Decimal> = taxed.iter().map(|v| round2(*v)).collect();
    // Derived from the *rounded* pair, not rounded independently, so
    // `gross == net + tax` holds pointwise on screen.
    let net_withdrawals_series: Vec<Decimal> = withdrawals_series
        .iter()
        .zip(tax_series.iter())
        .map(|(w, t)| *w - *t)
        .collect();
    let tax_paid_total = round2(tax_total);
    let net_withdrawn_total = withdrawn_total - tax_paid_total;
    let effective_tax_rate = tax_paid_total
        .checked_div(withdrawn_total)
        .unwrap_or(Decimal::ZERO);

    Ok(CalcOutput {
        investments: results,
        series,
        contributions_series,
        withdrawals_series,
        horizon_months,
        drawdown_months,
        total_months,
        current_total,
        handover_total,
        contributed_total,
        withdrawn_total,
        depletion_month,
        projected_total,
        growth,
        growth_pct,
        deployed,
        tax_series,
        net_withdrawals_series,
        tax_paid_total,
        net_withdrawn_total,
        effective_tax_rate,
        tax_by_period,
        unused_allowance_total,
        accounts_touched,
        accounts_touched_typical,
        rate_cap_breached,
        rules_label,
        rules_as_of,
    })
}

/// What to solve the projection *for*. The two deposits-mode goals carry the
/// user's target as a raw string, parsed here the way [`calculate`] parses
/// inputs. The two drawdown-mode goals carry nothing: the drawdown period and the
/// monthly withdrawal they reason about are already in [`CalcInput`], so there is
/// no extra number to pass. Every goal is measured against the whole portfolio.
pub enum Goal {
    /// Solve for the recurring monthly deposit that makes the portfolio reach
    /// `target` over the horizon. Deposits mode.
    MonthlyTopUp { target: String },
    /// Solve for the time, in whole months, until the portfolio first reaches
    /// `target` at its current rates and deposits. Deposits mode.
    TimeToTarget { target: String },
    /// Solve for the largest monthly withdrawal the portfolio can sustain and
    /// still reach exactly £0 at the end of the drawdown period in the input.
    /// Drawdown mode.
    MaxWithdrawal,
    /// Solve for how long, in whole months *of drawdown*, the monthly withdrawal
    /// in the input lasts before the portfolio runs dry. Drawdown mode.
    TimeToDeplete,
}

/// The answer to a [`Goal`].
#[derive(Clone, Debug, PartialEq)]
pub enum Solution {
    /// The monthly deposit required, rounded up to the penny so it genuinely
    /// reaches the target rather than landing a fraction short.
    MonthlyTopUp(Decimal),
    /// Whole months until the portfolio first reaches the target.
    Months(u32),
    /// The target is already met by the inputs as they stand — no deposit needed,
    /// or the portfolio is already worth at least the target today.
    AlreadyMet,
    /// The largest sustainable monthly withdrawal, rounded *down* to the penny so
    /// the figure reported is one the pot genuinely supports.
    MaxWithdrawal(Decimal),
    /// Whole months *of drawdown* until the portfolio runs dry under the
    /// withdrawal asked about. This counts from the start of drawdown, not from
    /// today — [`CalcOutput::depletion_month`] is the absolute index.
    Depletes(u32),
    /// The returns cover the withdrawal, so the pot never runs dry within the
    /// 100-year cap — there is no month to report rather than a very large one.
    NeverDepletes,
}

/// The largest monthly figure the bracket search will consider before declaring a
/// target unreachable, for both the top-up and the withdrawal searches. A billion
/// a month is comfortably past any real use and keeps the doubling bounded.
const MAX_TOP_UP: i64 = 1_000_000_000;

/// The 100-year projection cap, in months. `calculate` rejects any period past
/// this, and the time-based solvers project out to exactly it.
const MAX_HORIZON_MONTHS: u32 = 1200;

/// Solve the projection for a [`Goal`]. Shares [`calculate`]'s never-panic
/// contract: invalid input and unreachable targets come back as `Err`, never a
/// panic or a hang, and all arithmetic stays exact `Decimal`.
pub fn solve(input: &CalcInput, goal: &Goal) -> Result<Solution, CalcError> {
    match goal {
        Goal::MonthlyTopUp { target } => solve_top_up(input, target),
        Goal::TimeToTarget { target } => solve_time(input, target),
        Goal::MaxWithdrawal => solve_max_withdrawal(input),
        Goal::TimeToDeplete => solve_time_to_deplete(input),
    }
}

/// A goal needs at least one holding to be about.
fn require_holdings(input: &CalcInput) -> Result<(), CalcError> {
    if input.investments.is_empty() {
        Err(CalcError::new("Add a holding before solving a goal.", None))
    } else {
        Ok(())
    }
}

/// The target field of a goal is not one of the form's investment controls, so
/// its errors carry no `Field` — they surface as a portfolio-level message.
fn parse_target(target: &str) -> Result<Decimal, CalcError> {
    let t = parse_number(target)
        .ok_or_else(|| CalcError::new("Enter a valid target amount.", None))?;
    if t < Decimal::ZERO {
        return Err(CalcError::new("The target amount cannot be negative.", None));
    }
    Ok(t)
}

/// Spread a monthly deposit of `amount` evenly across `investments`: each row's
/// `contribution` becomes `amount / len`. A single holding takes the whole
/// amount. The one place the "a portfolio top-up is shared equally" convention
/// lives.
fn spread_deposit(investments: &mut [InvestmentInput], amount: Decimal) {
    let each = amount / Decimal::from(investments.len());
    for inv in investments {
        inv.contribution = each.to_string();
    }
}

/// A deposits-only clone of `input` over the 100-year cap — the base for both
/// deposits-mode solvers, which scan or bracket a long accumulation regardless of
/// the horizon (or drawdown plan) the user actually entered.
fn long_deposits(input: &CalcInput) -> CalcInput {
    CalcInput {
        investments: input.investments.clone(),
        horizon_value: MAX_HORIZON_MONTHS.to_string(),
        horizon_unit: Unit::Months,
        plan: Plan::Deposits,
        tax: input.tax.clone(),
    }
}

/// The portfolio value a candidate monthly top-up reaches over the *user's*
/// horizon, as a deposit split evenly across the holdings. Rises monotonically
/// with `top_up`, which is what lets the bracket search converge.
fn projected_under(input: &CalcInput, top_up: Decimal) -> Result<Decimal, CalcError> {
    let mut probe = input.clone();
    probe.plan = Plan::Deposits; // a top-up is a deposit, whatever mode we came from
    spread_deposit(&mut probe.investments, top_up);
    Ok(calculate(&probe)?.projected_total)
}

/// Bisection on the monthly top-up. `projected_under` rises monotonically with
/// the top-up, so a doubling bracket plus binary search converges on the least
/// top-up that reaches the target.
fn solve_top_up(input: &CalcInput, target: &str) -> Result<Solution, CalcError> {
    let target = parse_target(target)?;
    require_holdings(input)?;

    let projected_with = |top_up: Decimal| projected_under(input, top_up);

    if projected_with(Decimal::ZERO)? >= target {
        return Ok(Solution::AlreadyMet);
    }

    let cap = Decimal::from(MAX_TOP_UP);
    let mut hi = Decimal::ONE;
    while projected_with(hi)? < target {
        hi *= Decimal::from(2u32);
        if hi > cap {
            return Err(CalcError::new(
                format!(
                    "No monthly top-up reaches {} in this time; extend the horizon or lower the target.",
                    fmt_money_plain(target)
                ),
                None,
            ));
        }
    }

    let mut lo = Decimal::ZERO;
    let cent = Decimal::new(1, 2); // 0.01
    for _ in 0..80 {
        if hi - lo <= cent {
            break;
        }
        let mid = (lo + hi) / Decimal::from(2u32);
        if projected_with(mid)? >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    let answer = hi.round_dp_with_strategy(2, RoundingStrategy::AwayFromZero);
    Ok(Solution::MonthlyTopUp(answer))
}

/// Time until the portfolio reaches the target. With annualised-only rates, a
/// row's rate no longer depends on the horizon, so this simply projects the
/// deposits over the 100-year cap and scans the series for the first month that
/// clears the target.
fn solve_time(input: &CalcInput, target: &str) -> Result<Solution, CalcError> {
    let target = parse_target(target)?;
    require_holdings(input)?;

    let out = calculate(&long_deposits(input))?;

    match out.series.iter().position(|v| *v >= target) {
        Some(0) => Ok(Solution::AlreadyMet),
        Some(i) => Ok(Solution::Months(i as u32)),
        None => Err(CalcError::new(
            format!(
                "The portfolio does not reach {} within 100 years; raise the returns or the contributions.",
                fmt_money_plain(target)
            ),
            None,
        )),
    }
}

/// Clone `input` with the drawdown withdrawal set to `w`, keeping its accumulation
/// horizon and drawdown period. The primitive both drawdown solvers probe.
fn with_withdrawal(input: &CalcInput, w: Decimal) -> Result<CalcInput, CalcError> {
    // Clone and overwrite the one field that varies, rather than re-listing
    // every other: a new `CalcInput` or `Plan::Drawdown` field would otherwise
    // have to be threaded through here to avoid being silently dropped.
    let mut probe = input.clone();
    let Plan::Drawdown { withdrawal, .. } = &mut probe.plan else {
        return Err(CalcError::new(
            "This goal only applies while drawing the portfolio down.",
            None,
        ));
    };
    *withdrawal = w.to_string();
    Ok(probe)
}

/// Bisection on the monthly withdrawal: the largest draw that still leaves the
/// portfolio solvent to the end of the drawdown period (i.e. reaches £0 no sooner
/// than the final month). Feasibility is downward-closed — drawing less is always
/// at least as safe — so the feasible side of the bracket is the low one and the
/// answer is rounded down.
fn solve_max_withdrawal(input: &CalcInput) -> Result<Solution, CalcError> {
    require_holdings(input)?;

    // The pot at the start of drawdown, drawing nothing. If it is already empty
    // there is nothing to spend down.
    let base = calculate(&with_withdrawal(input, Decimal::ZERO)?)?;
    if base.handover_total.is_some_and(|p| p <= Decimal::ZERO) {
        return Err(CalcError::new(
            "The portfolio has nothing left to draw down at the end of the growth period.",
            None,
        ));
    }

    // A draw is feasible if it survives the whole drawdown period without the
    // portfolio ever hitting £0.
    let feasible = |w: Decimal| -> Result<bool, CalcError> {
        Ok(calculate(&with_withdrawal(input, w)?)?.depletion_month.is_none())
    };

    if !feasible(Decimal::ZERO)? {
        return Err(CalcError::new(
            "The portfolio runs dry before the drawdown ends even with no withdrawals.",
            None,
        ));
    }

    let cap = Decimal::from(MAX_TOP_UP);
    let mut hi = Decimal::ONE;
    while feasible(hi)? {
        if hi >= cap {
            return Err(CalcError::new(
                format!(
                    "The portfolio can sustain more than {} a month, which is beyond the range this solver reports.",
                    fmt_money_plain(cap)
                ),
                None,
            ));
        }
        hi = (hi * Decimal::from(2u32)).min(cap);
    }

    let mut lo = Decimal::ZERO;
    let cent = Decimal::new(1, 2); // 0.01
    for _ in 0..80 {
        if hi - lo <= cent {
            break;
        }
        let mid = (lo + hi) / Decimal::from(2u32);
        if feasible(mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    // Round down, then settle the last penny against `calculate` itself: step up
    // while the next penny still holds. Bounded regardless — the never-hang
    // contract outranks the extra penny.
    let mut answer = lo.round_dp_with_strategy(2, RoundingStrategy::ToZero);
    for _ in 0..3 {
        if !feasible(answer + cent)? {
            break;
        }
        answer += cent;
    }

    // Under pro-rata, feasibility is downward-closed and the figure above always
    // holds. Under a tax-aware order it is only *empirically* so — drawing more
    // this month changes which account is cheapest next month, so the balances
    // are not pointwise ordered and the bisection can settle a hair too high.
    // Rather than assert a monotonicity that cannot be proved, verify and step
    // back down. Bounded, so it can fail loudly but never hang.
    for _ in 0..8 {
        if feasible(answer)? {
            break;
        }
        answer -= cent;
    }
    if !feasible(answer)? {
        return Err(CalcError::new(
            "Could not settle on a sustainable withdrawal for this withdrawal order.",
            None,
        ));
    }

    Ok(Solution::MaxWithdrawal(answer))
}

/// How long the drawdown withdrawal lasts. Re-projects the drawdown over the
/// whole remaining span to the 100-year cap (the pot may outlast the period on
/// screen), reads the month the portfolio first hits £0 off [`calculate`], and
/// reports it relative to the *start of drawdown*.
fn solve_time_to_deplete(input: &CalcInput) -> Result<Solution, CalcError> {
    require_holdings(input)?;

    let horizon_months = to_months(&input.horizon_value, input.horizon_unit, "The growth period")
        .map_err(|m| CalcError::new(m, Some(Field::Horizon)))?;
    // No room left to draw down within the cap: nothing can run out.
    if horizon_months >= MAX_HORIZON_MONTHS {
        return Ok(Solution::NeverDepletes);
    }
    let span = MAX_HORIZON_MONTHS - horizon_months;

    // Project the *user's* withdrawal over the long span. `with_withdrawal`
    // carries the plan's withdrawal through unchanged; we only lengthen the
    // period. A blank/zero draw never depletes.
    let long = CalcInput {
        investments: input.investments.clone(),
        horizon_value: input.horizon_value.clone(),
        horizon_unit: input.horizon_unit,
        plan: match &input.plan {
            Plan::Drawdown { withdrawal, strategy, .. } => Plan::Drawdown {
                drawdown_value: span.to_string(),
                drawdown_unit: Unit::Months,
                withdrawal: withdrawal.clone(),
                strategy: strategy.clone(),
            },
            Plan::Deposits => {
                return Err(CalcError::new("This goal only applies while drawing the portfolio down.", None))
            }
        },
        tax: input.tax.clone(),
    };
    let out = calculate(&long)?;

    Ok(match out.depletion_month {
        Some(m) => Solution::Depletes(m - horizon_months),
        None => Solution::NeverDepletes,
    })
}

/// A grouped `£1,234.56` for embedding in error messages, matching the UI's
/// `fmt_money`. Kept here (not in the UI `format` module) because `calc` owns
/// its own message text.
fn fmt_money_plain(d: Decimal) -> String {
    let s = format!("{:.2}", d.round_dp(2).abs());
    let (int, frac) = s.split_once('.').unwrap_or((s.as_str(), "00"));
    let len = int.len();
    let mut grouped = String::with_capacity(len + len / 3);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if d.is_sign_negative() { "-" } else { "" };
    format!("{sign}\u{00a3}{grouped}.{frac}")
}

/// Convert a `value` + `unit` into whole months, doing the ×12 and the rounding
/// of fractional periods entirely in `Decimal`. `field` names the input for
/// error messages.
fn to_months(value: &str, unit: Unit, field: &str) -> Result<u32, String> {
    let v = Decimal::from_str(value.trim())
        .map_err(|_| format!("{field} is not a valid number."))?;
    if v.is_sign_negative() {
        return Err(format!("{field} cannot be negative."));
    }
    let months = match unit {
        Unit::Months => v,
        Unit::Years => v
            .checked_mul(Decimal::from(12u32))
            .ok_or_else(|| format!("{field} is too large."))?,
    };
    months
        .round()
        .to_u32()
        .ok_or_else(|| format!("{field} is too large."))
}

/// Parse a number the way a user actually types one: `1,234.56`, `£1,234`,
/// `7 %` and `1 234` all parse. Grouping separators, whitespace, a leading
/// currency symbol and a trailing percent sign are noise here — the field
/// already tells us what the number means.
///
/// This only widens what is accepted; it never guesses. Anything that is not a
/// plain decimal number once the noise is gone still fails. Note the en-GB
/// assumption baked in: `,` is a thousands separator, not a decimal point.
pub fn parse_number(s: &str) -> Option<Decimal> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '_' | '\u{00a3}' | '$' | '\u{20ac}' | '%'))
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    Decimal::from_str(&cleaned).ok()
}

fn round2(d: Decimal) -> Decimal {
    d.round_dp(2)
}

fn too_large_msg(name: &str) -> String {
    format!("'{name}' grows too large to project; lower the return or the horizon.")
}

fn portfolio_too_large() -> String {
    "The portfolio total is too large to project; lower the values or the horizon.".to_string()
}

/// The portfolio-level overflow error, ready to hand to `ok_or_else`.
///
/// Named because it is raised from a dozen arithmetic sites; spelling the
/// `CalcError` out at each one meant the same sentence could drift between
/// them, and the closure was longer than the arithmetic it guarded.
fn overflowed() -> CalcError {
    CalcError::new(portfolio_too_large(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    // --- builders ----------------------------------------------------------

    fn holding(name: &str, value: &str, rate: &str, contribution: &str) -> InvestmentInput {
        InvestmentInput {
            name: name.into(),
            value: value.into(),
            rate: rate.into(),
            contribution: contribution.into(),
            ..Default::default()
        }
    }

    /// A holding in a named account, for the tax-aware drawdown tests. `basis`
    /// is only consulted by kinds taxed on the gain.
    fn account(name: &str, kind: &str, value: &str, rate: &str, basis: &str) -> InvestmentInput {
        InvestmentInput {
            name: name.into(),
            value: value.into(),
            rate: rate.into(),
            contribution: "0".into(),
            account_kind: kind.into(),
            cost_basis: basis.into(),
        }
    }

    fn deposits(investments: Vec<InvestmentInput>, horizon: &str, hunit: Unit) -> CalcInput {
        CalcInput {
            investments,
            horizon_value: horizon.into(),
            horizon_unit: hunit,
            plan: Plan::Deposits,
            tax: None,
        }
    }

    fn one(value: &str, rate: &str, horizon: &str, hunit: Unit) -> CalcInput {
        deposits(vec![holding("X", value, rate, "0")], horizon, hunit)
    }

    fn with_contribution(value: &str, rate: &str, contribution: &str, horizon: &str, hunit: Unit) -> CalcInput {
        deposits(vec![holding("X", value, rate, contribution)], horizon, hunit)
    }

    /// A drawdown input: grow the holdings for `grow`/`gunit`, then draw
    /// `withdrawal` a month for `draw`/`dunit`.
    fn drawdown(
        investments: Vec<InvestmentInput>,
        grow: &str,
        gunit: Unit,
        draw: &str,
        dunit: Unit,
        withdrawal: &str,
    ) -> CalcInput {
        CalcInput {
            investments,
            horizon_value: grow.into(),
            horizon_unit: gunit,
            plan: Plan::Drawdown {
                drawdown_value: draw.into(),
                drawdown_unit: dunit,
                withdrawal: withdrawal.into(),
                strategy: Strategy::ProRata,
            },
            tax: None,
        }
    }

    /// A tax context over the mock system: a fictional jurisdiction with round
    /// numbers, so these tests pin the *engine* rather than one country's
    /// figures. A rate change in `uktax` must not be able to break `calc`.
    fn taxed(other_income: &str, age: &str) -> TaxContext {
        TaxContext {
            system: &taxkit::mock::MOCK,
            region: "all".into(),
            other_income: other_income.into(),
            age: age.into(),
            uprate: "0".into(),
        }
    }

    /// A drawdown under `strategy`, taxed by the mock system.
    fn strategy_run(
        investments: Vec<InvestmentInput>,
        grow: &str,
        draw: &str,
        withdrawal: &str,
        strategy: Strategy,
        tax: Option<TaxContext>,
    ) -> CalcInput {
        CalcInput {
            investments,
            horizon_value: grow.into(),
            horizon_unit: Unit::Months,
            plan: Plan::Drawdown {
                drawdown_value: draw.into(),
                drawdown_unit: Unit::Months,
                withdrawal: withdrawal.into(),
                strategy,
            },
            tax,
        }
    }

    // --- deposits: accumulation --------------------------------------------

    #[test]
    fn annualised_projection_matches_hand_calculation() {
        let out = calculate(&one("10000", "7", "10", Unit::Years)).unwrap();
        assert_eq!(out.horizon_months, 120);
        assert_eq!(out.total_months, 120);
        assert_eq!(out.drawdown_months, 0);
        assert_eq!(out.handover_total, None);
        assert_eq!(out.investments[0].current_value, d("10000.00"));
        assert_eq!(out.investments[0].projected_value, d("19671.51"));
        assert_eq!(out.investments[0].handover_value, None);
        assert_eq!(out.current_total, d("10000.00"));
    }

    #[test]
    fn years_and_months_agree() {
        let a = calculate(&one("100", "7", "3", Unit::Years)).unwrap();
        let b = calculate(&one("100", "7", "36", Unit::Months)).unwrap();
        assert_eq!(a.projected_total, b.projected_total);
    }

    #[test]
    fn fractional_years_round_to_whole_months_in_decimal() {
        let out = calculate(&one("100", "0", "1.1", Unit::Years)).unwrap();
        assert_eq!(out.horizon_months, 13);
    }

    #[test]
    fn zero_return_leaves_value_unchanged() {
        let out = calculate(&one("500", "0", "5", Unit::Years)).unwrap();
        assert_eq!(out.current_total, d("500.00"));
        assert_eq!(out.projected_total, d("500.00"));
        assert_eq!(out.growth, d("0.00"));
    }

    #[test]
    fn guards_reject_bad_input() {
        assert!(calculate(&one("100", "7", "0", Unit::Months))
            .unwrap_err()
            .message
            .contains("at least 1 month"));
        assert!(calculate(&one("100", "-150", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("-100%"));
        assert!(calculate(&one("-100", "7", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("negative amount"));
        assert!(calculate(&one("abc", "7", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("invalid amount"));
    }

    #[test]
    fn accepts_the_ways_people_actually_type_numbers() {
        let grouped = calculate(&one("10,000", "7", "10", Unit::Years)).unwrap();
        let plain = calculate(&one("10000", "7", "10", Unit::Years)).unwrap();
        assert_eq!(grouped, plain);

        for value in ["\u{00a3}10,000", " 10000 ", "10 000", "\u{00a3} 10,000.00"] {
            assert_eq!(
                calculate(&one(value, "7", "10", Unit::Years)).unwrap(),
                plain,
                "{value} should parse as 10000"
            );
        }
        assert_eq!(calculate(&one("10000", "7%", "10", Unit::Years)).unwrap(), plain);
        assert_eq!(
            calculate(&with_contribution("10000", "7", "1,000", "10", Unit::Years))
                .unwrap()
                .contributed_total,
            d("120000.00")
        );

        assert_eq!(
            calculate(&one("1,234.56", "7", "10", Unit::Years)).unwrap().current_total,
            d("1234.56")
        );
        assert!(calculate(&one("-1,000", "7", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("negative amount"));
    }

    #[test]
    fn lenient_parsing_still_rejects_nonsense() {
        for bad in ["abc", "1.2.3", "--5", "\u{00a3}", "1/2", ""] {
            assert!(
                calculate(&one(bad, "7", "10", Unit::Years)).is_err(),
                "{bad:?} should not parse"
            );
        }
    }

    #[test]
    fn errors_point_at_the_field_that_caused_them() {
        use InvestmentField::{Contribution, Rate, Value};
        let field = |i: &CalcInput| calculate(i).unwrap_err().field;

        assert_eq!(
            field(&one("abc", "7", "10", Unit::Years)),
            Some(Field::Investment { index: 0, part: Value })
        );
        assert_eq!(
            field(&one("100", "abc", "10", Unit::Years)),
            Some(Field::Investment { index: 0, part: Rate })
        );
        assert_eq!(
            field(&with_contribution("100", "7", "-5", "10", Unit::Years)),
            Some(Field::Investment { index: 0, part: Contribution })
        );
        assert_eq!(field(&one("100", "7", "0", Unit::Months)), Some(Field::Horizon));
        assert_eq!(
            field(&CalcInput {
                investments: vec![],
                horizon_value: "10".into(),
                horizon_unit: Unit::Years,
                plan: Plan::Deposits,
                tax: None,
            }),
            None
        );
    }

    #[test]
    fn error_index_identifies_which_row_failed() {
        let input = deposits(
            vec![
                holding("A", "10000", "7", "0"),
                holding("B", "5000", "7", "0"),
                holding("C", "oops", "7", "0"),
            ],
            "10",
            Unit::Years,
        );
        let err = calculate(&input).unwrap_err();
        assert_eq!(err.field, Some(Field::Investment { index: 2, part: InvestmentField::Value }));
        assert!(err.message.contains('C'));
    }

    #[test]
    fn extreme_growth_errors_instead_of_panicking() {
        let out = calculate(&one("10000", "100", "100", Unit::Years));
        assert!(out.unwrap_err().message.contains("too large"));
    }

    #[test]
    fn huge_horizon_in_years_errors_instead_of_panicking() {
        let out = calculate(&one("100", "7", "9999999999999999999999999999", Unit::Years));
        assert!(out.unwrap_err().message.contains("too large"));
    }

    #[test]
    fn horizon_over_100_years_is_rejected() {
        assert!(calculate(&one("100", "7", "101", Unit::Years))
            .unwrap_err()
            .message
            .contains("100 years"));
    }

    #[test]
    fn portfolio_sums_across_investments() {
        let input = deposits(
            vec![holding("A", "10000", "7", "0"), holding("B", "5000", "0", "0")],
            "10",
            Unit::Years,
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.current_total, d("15000.00"));
        // A grows to 19,671.51; B is flat at 5,000.
        assert_eq!(out.projected_total, d("24671.51"));
        assert_eq!(out.series.len(), 121);
        assert_eq!(*out.series.first().unwrap(), out.current_total);
        assert_eq!(*out.series.last().unwrap(), out.projected_total);
    }

    #[test]
    fn contributions_add_up_and_are_excluded_from_growth() {
        let out = calculate(&with_contribution("1000", "0", "100", "12", Unit::Months)).unwrap();
        assert_eq!(out.current_total, d("1000.00"));
        assert_eq!(out.contributed_total, d("1200.00"));
        assert_eq!(out.projected_total, d("2200.00"));
        assert_eq!(out.growth, d("0"));
        assert_eq!(out.growth_pct, d("0"));
        assert_eq!(out.deployed, d("2200.00"));
    }

    #[test]
    fn deployed_is_the_denominator_of_growth_pct() {
        let out = calculate(&with_contribution("10000", "7", "200", "10", Unit::Years)).unwrap();
        assert_eq!(out.deployed, out.current_total + out.contributed_total);
        assert_eq!(out.deployed, d("34000.00"));
        assert_eq!((out.growth / out.deployed).round_dp(6), out.growth_pct.round_dp(6));
    }

    #[test]
    fn per_row_contributed_is_that_rows_own_top_ups() {
        let input = deposits(
            vec![holding("A", "10000", "7", "200"), holding("B", "5000", "0", "0")],
            "10",
            Unit::Years,
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.investments[0].contributed, d("24000.00"));
        assert_eq!(out.investments[1].contributed, d("0.00"));
        assert_eq!(
            out.investments.iter().map(|r| r.contributed).sum::<Decimal>(),
            out.contributed_total
        );
    }

    #[test]
    fn contributed_reconciles_a_row_that_value_alone_cannot() {
        let out = calculate(&with_contribution("10000", "7", "200", "10", Unit::Years)).unwrap();
        let row = &out.investments[0];
        assert_eq!(row.current_value, d("10000.00"));
        assert_eq!(row.contributed, d("24000.00"));
        assert_eq!(row.projected_value, d("53881.86"));
        assert!(row.projected_value > row.current_value + row.contributed);
    }

    #[test]
    fn contributions_series_accumulates_month_by_month() {
        let out = calculate(&with_contribution("1000", "0", "100", "12", Unit::Months)).unwrap();
        assert_eq!(out.contributions_series.len(), out.series.len());
        assert_eq!(out.contributions_series[0], d("0.00"));
        assert_eq!(out.contributions_series[1], d("100.00"));
        assert_eq!(out.contributions_series[6], d("600.00"));
        assert_eq!(*out.contributions_series.last().unwrap(), out.contributed_total);
    }

    #[test]
    fn contributions_series_is_all_zero_without_top_ups() {
        let out = calculate(&one("1000", "12", "24", Unit::Months)).unwrap();
        assert!(out.contributions_series.iter().all(|c| c.is_zero()));
    }

    #[test]
    fn withdrawals_series_is_all_zero_in_deposits_mode() {
        let out = calculate(&with_contribution("1000", "5", "50", "24", Unit::Months)).unwrap();
        assert!(out.withdrawals_series.iter().all(|w| w.is_zero()));
        assert_eq!(out.withdrawn_total, d("0.00"));
    }

    #[test]
    fn contributions_increase_the_projection_but_not_today() {
        let base = calculate(&one("1000", "12", "24", Unit::Months)).unwrap();
        let with = calculate(&with_contribution("1000", "12", "50", "24", Unit::Months)).unwrap();
        assert!(with.projected_total > base.projected_total);
        assert_eq!(with.contributed_total, d("1200.00"));
        assert_eq!(with.series[0], base.series[0]);
        assert_eq!(with.current_total, base.current_total);
    }

    #[test]
    fn portfolio_summary_overflow_errors_instead_of_panicking() {
        let out = calculate(&with_contribution(
            "79000000000000000000000000000",
            "-50",
            "10000000000000000000000000",
            "1200",
            Unit::Months,
        ));
        assert!(out.unwrap_err().message.contains("too large"));
    }

    #[test]
    fn zero_deployed_capital_reports_zero_growth_pct() {
        let out = calculate(&one("0", "7", "10", Unit::Years)).unwrap();
        assert_eq!(out.current_total, d("0.00"));
        assert_eq!(out.growth_pct, Decimal::ZERO);
    }

    #[test]
    fn negative_contribution_is_rejected() {
        assert!(calculate(&with_contribution("1000", "5", "-50", "10", Unit::Years))
            .unwrap_err()
            .message
            .contains("negative monthly amount"));
    }

    // --- drawdown: two-phase projection ------------------------------------

    #[test]
    fn handover_is_the_accumulation_projection() {
        // The pot at the start of drawdown must equal, to the penny, what the same
        // holdings project to as a plain deposits run over the accumulation
        // period — and the whole accumulation slice of the series must match.
        let holdings = vec![holding("Eq", "10000", "7", "200"), holding("Bond", "5000", "3", "0")];
        let acc = calculate(&deposits(holdings.clone(), "10", Unit::Years)).unwrap();
        let dd = calculate(&drawdown(holdings, "10", Unit::Years, "30", Unit::Years, "2000")).unwrap();

        assert_eq!(dd.handover_total, Some(acc.projected_total));
        assert_eq!(dd.series[..=120], acc.series[..]);
        for (a, b) in dd.investments.iter().zip(acc.investments.iter()) {
            assert_eq!(a.handover_value, Some(b.projected_value));
        }
    }

    #[test]
    fn series_spans_both_phases_and_deposits_stop_at_handover() {
        let dd = calculate(&drawdown(
            vec![holding("X", "10000", "5", "100")],
            "10",
            Unit::Years,
            "20",
            Unit::Years,
            "300",
        ))
        .unwrap();
        assert_eq!(dd.horizon_months, 120);
        assert_eq!(dd.drawdown_months, 240);
        assert_eq!(dd.total_months, 360);
        assert_eq!(dd.series.len(), 361);
        assert_eq!(dd.series[120], dd.handover_total.unwrap());
        // Deposits are flat from the handover on.
        let paid_at_handover = dd.contributions_series[120];
        assert!(dd.contributions_series[360..].iter().all(|c| *c == paid_at_handover));
        assert_eq!(paid_at_handover, d("12000.00")); // 120 * 100
    }

    #[test]
    fn withdrawals_start_the_month_after_handover() {
        // 0% rates so the arithmetic is exact: pot at handover is P, first
        // withdrawal lands at index A+1.
        let dd = calculate(&drawdown(
            vec![holding("X", "12000", "0", "0")],
            "12",
            Unit::Months,
            "12",
            Unit::Months,
            "500",
        ))
        .unwrap();
        let p = dd.handover_total.unwrap();
        assert_eq!(p, d("12000.00"));
        assert_eq!(dd.series[12], d("12000.00"));
        assert_eq!(dd.series[13], d("11500.00"));
        assert_eq!(dd.withdrawals_series[12], d("0.00"));
        assert_eq!(dd.withdrawals_series[13], d("500.00"));
    }

    #[test]
    fn pro_rata_split_is_by_current_value() {
        // £3,000 and £1,000 at 0%, draw £400: month 1 takes £300 / £100.
        let dd = calculate(&drawdown(
            vec![holding("Big", "3000", "0", "0"), holding("Small", "1000", "0", "0")],
            "1",
            Unit::Months,
            "1",
            Unit::Months,
            "400",
        ))
        .unwrap();
        assert_eq!(dd.investments[0].withdrawn, d("300.00"));
        assert_eq!(dd.investments[1].withdrawn, d("100.00"));
        assert_eq!(dd.withdrawn_total, d("400.00"));
    }

    #[test]
    fn pro_rata_split_follows_the_growing_holding() {
        // Same start, different rates: over a long draw the higher-return holding
        // funds a growing share of each withdrawal.
        let dd = calculate(&drawdown(
            vec![holding("Fast", "10000", "12", "0"), holding("Slow", "10000", "0", "0")],
            "1",
            Unit::Months,
            "120",
            Unit::Months,
            "100",
        ))
        .unwrap();
        // The fast holding is worth more by the end, so it has funded more of the draw.
        assert!(dd.investments[0].withdrawn > dd.investments[1].withdrawn);
        // Per-row withdrawals still reconcile to the portfolio figure exactly.
        assert_eq!(
            dd.investments.iter().map(|r| r.withdrawn).sum::<Decimal>(),
            dd.withdrawn_total
        );
    }

    #[test]
    fn every_holding_empties_in_the_same_month_and_reconciles() {
        // Two holdings at 0%; a portfolio draw that empties them. Under monthly
        // pro-rata they run dry together, and the per-row draws sum exactly.
        let dd = calculate(&drawdown(
            vec![holding("A", "6000", "0", "0"), holding("B", "6000", "0", "0")],
            "1",
            Unit::Months,
            "24",
            Unit::Months,
            "1000",
        ))
        .unwrap();
        // £12,000 at £1,000/mo from month 2 onward: gone at absolute month 13.
        assert_eq!(dd.depletion_month, Some(13));
        assert_eq!(
            dd.investments.iter().map(|r| r.withdrawn).sum::<Decimal>(),
            dd.withdrawn_total
        );
        assert_eq!(dd.projected_total, d("0.00"));
    }

    #[test]
    fn portfolio_withdrawal_is_capped_at_the_pot() {
        // £1,000 pot, drawing £600 a month at 0%: month 1 leaves £400, month 2
        // can only take that £400, so total withdrawn is £1,000, not £1,200.
        let dd = calculate(&drawdown(
            vec![holding("X", "1000", "0", "0")],
            "1",
            Unit::Months,
            "6",
            Unit::Months,
            "600",
        ))
        .unwrap();
        assert_eq!(dd.withdrawn_total, d("1000.00"));
        assert_eq!(dd.projected_total, d("0.00"));
    }

    #[test]
    fn withdrawn_total_is_exactly_the_amount_asked_until_dry() {
        // 0% rates: three uneven holdings, a draw that does not empty the pot, over
        // 10 months. The reported total must be exactly 10 * the monthly draw with
        // no rounding drift, and the per-row shares must sum to it.
        let dd = calculate(&drawdown(
            vec![
                holding("A", "1000", "0", "0"),
                holding("B", "3000", "0", "0"),
                holding("C", "7000", "0", "0"),
            ],
            "1",
            Unit::Months,
            "10",
            Unit::Months,
            "333.33",
        ))
        .unwrap();
        assert_eq!(dd.withdrawn_total, d("3333.30")); // 10 * 333.33, exact
        assert_eq!(
            dd.investments.iter().map(|r| r.withdrawn).sum::<Decimal>(),
            dd.withdrawn_total
        );
    }

    #[test]
    fn depletion_month_is_absolute_and_matches_the_series() {
        let dd = calculate(&drawdown(
            vec![holding("X", "1000", "0", "0")],
            "6",
            Unit::Months,
            "12",
            Unit::Months,
            "250",
        ))
        .unwrap();
        let m = dd.depletion_month.unwrap() as usize;
        assert_eq!(dd.series[m], d("0.00"));
        assert!(dd.series[m - 1] > Decimal::ZERO);
    }

    #[test]
    fn growth_adds_back_withdrawals_and_reconciles_over_two_phases() {
        // The reconciliation identity must hold exactly across a handover:
        // projected = current + deposits - withdrawals + growth.
        let dd = calculate(&drawdown(
            vec![holding("Eq", "10000", "7", "200"), holding("Bond", "5000", "3", "0")],
            "10",
            Unit::Years,
            "30",
            Unit::Years,
            "2000",
        ))
        .unwrap();
        assert!(dd.withdrawn_total > Decimal::ZERO);
        assert_eq!(
            dd.projected_total,
            dd.current_total + dd.contributed_total - dd.withdrawn_total + dd.growth
        );
    }

    #[test]
    fn validation_names_the_new_controls() {
        // Bad drawdown period -> Field::Drawdown.
        assert_eq!(
            calculate(&drawdown(vec![holding("X", "1000", "5", "0")], "10", Unit::Years, "abc", Unit::Years, "100"))
                .unwrap_err()
                .field,
            Some(Field::Drawdown)
        );
        // Zero drawdown period -> Field::Drawdown.
        assert_eq!(
            calculate(&drawdown(vec![holding("X", "1000", "5", "0")], "10", Unit::Years, "0", Unit::Months, "100"))
                .unwrap_err()
                .field,
            Some(Field::Drawdown)
        );
        // Combined periods over the cap -> Field::Drawdown.
        assert_eq!(
            calculate(&drawdown(vec![holding("X", "1000", "5", "0")], "90", Unit::Years, "90", Unit::Years, "100"))
                .unwrap_err()
                .field,
            Some(Field::Drawdown)
        );
        // Bad withdrawal -> Field::Withdrawal.
        assert_eq!(
            calculate(&drawdown(vec![holding("X", "1000", "5", "0")], "10", Unit::Years, "10", Unit::Years, "abc"))
                .unwrap_err()
                .field,
            Some(Field::Withdrawal)
        );
        // Negative withdrawal -> Field::Withdrawal.
        assert_eq!(
            calculate(&drawdown(vec![holding("X", "1000", "5", "0")], "10", Unit::Years, "10", Unit::Years, "-5"))
                .unwrap_err()
                .field,
            Some(Field::Withdrawal)
        );
    }

    #[test]
    fn zero_withdrawal_is_a_flat_drawdown() {
        // A blank/zero draw is legal — the drawdown phase just keeps growing.
        let dd = calculate(&drawdown(
            vec![holding("X", "1000", "0", "0")],
            "1",
            Unit::Months,
            "12",
            Unit::Months,
            "0",
        ))
        .unwrap();
        assert_eq!(dd.withdrawn_total, d("0.00"));
        assert_eq!(dd.projected_total, d("1000.00"));
        assert_eq!(dd.depletion_month, None);
    }

    #[test]
    fn two_phase_overflow_errors_instead_of_panicking() {
        // 100% annualised over 50y grow + 50y draw. The pro-rata loop now has a
        // division per month; it must be checked and error, not panic.
        let out = calculate(&drawdown(
            vec![holding("X", "10000", "100", "0")],
            "50",
            Unit::Years,
            "50",
            Unit::Years,
            "100",
        ));
        assert!(out.unwrap_err().message.contains("too large"));
    }

    // --- drawdown: ordered and tax-aware strategies -------------------------
    //
    // These run against `taxkit`'s mock system, not against any real
    // jurisdiction. That is deliberate: it pins the *engine*, so a rate change
    // in `uktax` next April cannot break `calc`, and it is the second
    // implementation that proves nothing jurisdiction-specific leaked in here.

    use taxkit::mock::{FREE, GAINS, INCOME};

    /// Every strategy over the same portfolio, for the invariants that must hold
    /// under all of them.
    fn every_strategy() -> Vec<(&'static str, Strategy)> {
        vec![
            ("pro-rata", Strategy::ProRata),
            ("ordered", Strategy::Ordered { order: vec![GAINS.into(), FREE.into(), INCOME.into()] }),
            ("cheapest first", Strategy::CheapestFirst),
            ("preserve growth", Strategy::PreserveGrowth),
            ("rate capped", Strategy::RateCapped { max_rate: "20".into() }),
        ]
    }

    fn mixed_portfolio() -> Vec<InvestmentInput> {
        vec![
            account("Cash", FREE, "40000", "0", "0"),
            account("Unwrapped", GAINS, "40000", "0", "20000"),
            account("Pension", INCOME, "80000", "0", "0"),
        ]
    }

    #[test]
    fn pro_rata_ignores_the_tax_model_entirely() {
        // Invariant: an input that says nothing about tax, and an input that
        // does but splits pro-rata, must project *identically*. Pro-rata never
        // opens a session, which is what makes this structural rather than a
        // promise -- the tax-aware code is a separate path it never enters.
        let untaxed = strategy_run(mixed_portfolio(), "12", "60", "2000", Strategy::ProRata, None);
        let taxed_input = strategy_run(
            mixed_portfolio(),
            "12",
            "60",
            "2000",
            Strategy::ProRata,
            Some(taxed("30000", "60")),
        );
        assert_eq!(
            calculate(&untaxed).unwrap(),
            calculate(&taxed_input).unwrap(),
            "pro-rata must be byte-identical with and without a tax context"
        );
    }

    #[test]
    fn gross_is_always_net_plus_tax() {
        for (name, strategy) in every_strategy() {
            let input = strategy_run(
                mixed_portfolio(),
                "12",
                "120",
                "2000",
                strategy,
                Some(taxed("6000", "60")),
            );
            let out = calculate(&input).unwrap();

            for (i, ((gross, tax), net)) in out
                .withdrawals_series
                .iter()
                .zip(out.tax_series.iter())
                .zip(out.net_withdrawals_series.iter())
                .enumerate()
            {
                assert_eq!(*gross, *net + *tax, "{name}: month {i} must reconcile");
            }
            assert_eq!(
                out.withdrawn_total,
                out.net_withdrawn_total + out.tax_paid_total,
                "{name}: totals must reconcile"
            );
            for r in &out.investments {
                assert_eq!(
                    r.withdrawn,
                    r.net_withdrawn + r.tax_paid,
                    "{name}: '{}' must reconcile",
                    r.name
                );
            }
        }
    }

    #[test]
    fn per_row_figures_sum_exactly_to_the_portfolio_totals() {
        // The residue rule, generalised: every apportionment lands its rounding
        // on the last member of the group it apportions, so the rows still add
        // up exactly under an ordered strategy.
        for (name, strategy) in every_strategy() {
            let input = strategy_run(
                mixed_portfolio(),
                "12",
                "120",
                "2000",
                strategy,
                Some(taxed("6000", "60")),
            );
            let out = calculate(&input).unwrap();
            let rows_withdrawn: Decimal = out.investments.iter().map(|r| r.withdrawn).sum();
            let rows_tax: Decimal = out.investments.iter().map(|r| r.tax_paid).sum();
            assert_eq!(rows_withdrawn, out.withdrawn_total, "{name}: withdrawals");
            assert_eq!(rows_tax, out.tax_paid_total, "{name}: tax");
        }
    }

    #[test]
    fn the_reconciliation_identity_survives_every_strategy() {
        // projected = current + contributed - withdrawn + growth, with
        // `withdrawn` GROSS. Tax is a third flow and must not disturb this.
        for (name, strategy) in every_strategy() {
            let input = strategy_run(
                mixed_portfolio(),
                "12",
                "120",
                "2000",
                strategy,
                Some(taxed("6000", "60")),
            );
            let out = calculate(&input).unwrap();
            assert_eq!(
                out.projected_total,
                out.current_total + out.contributed_total - out.withdrawn_total + out.growth,
                "{name}: the identity must hold across the handover"
            );
        }
    }

    #[test]
    fn the_requested_net_is_delivered_in_full_while_the_pot_lasts() {
        for (name, strategy) in every_strategy() {
            if matches!(strategy, Strategy::ProRata) {
                continue; // pro-rata's withdrawal is gross, not net
            }
            let input = strategy_run(
                mixed_portfolio(),
                "12",
                "24",
                "2000",
                strategy,
                Some(taxed("6000", "60")),
            );
            let out = calculate(&input).unwrap();
            assert!(out.depletion_month.is_none(), "{name}: pot should survive this");
            assert_eq!(
                out.net_withdrawn_total.round_dp(2),
                Decimal::from(2000 * 24),
                "{name}: the holder asked for a net figure and must get it"
            );
        }
    }

    #[test]
    fn allowances_reset_only_at_a_period_boundary() {
        // The mock gives 12,000 of free income a period, and a period is twelve
        // months. Drawing 1,000 net a month therefore costs nothing at all --
        // in year one, and again in year two.
        let input = strategy_run(
            vec![account("Pension", INCOME, "120000", "0", "0")],
            "1",
            "24",
            "1000",
            Strategy::CheapestFirst,
            Some(taxed("0", "60")),
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.tax_paid_total, Decimal::ZERO, "each year's allowance covers it");
        assert_eq!(out.withdrawn_total, Decimal::from(24_000));
        assert_eq!(out.tax_by_period.len(), 2, "two tax periods");
        assert!(out.tax_by_period.iter().all(|t| t.is_zero()));
    }

    #[test]
    fn crossing_the_free_band_is_hand_checkable() {
        // 2,000 net a month against a 12,000 free band: six months free, then
        // six months grossed up at 20% (2,500 gross, 500 tax each).
        let input = strategy_run(
            vec![account("Pension", INCOME, "500000", "0", "0")],
            "1",
            "12",
            "2000",
            Strategy::CheapestFirst,
            Some(taxed("0", "60")),
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.withdrawn_total, Decimal::from(27_000), "12,000 free + 15,000 grossed up");
        assert_eq!(out.tax_paid_total, Decimal::from(3_000));
        assert_eq!(out.net_withdrawn_total, Decimal::from(24_000));
    }

    #[test]
    fn cheapest_first_spends_the_free_allowance_before_the_tax_free_account() {
        // The behaviour a tax-aware order exists for. An allowance and a
        // tax-free account both keep 100% of the next pound, so a chooser that
        // looks only at the marginal rate cannot tell them apart. But the
        // allowance expires each year and the account does not, so the allowance
        // must go first — otherwise it is wasted and the money comes out taxed
        // later instead.
        //
        // The tax-free pot has to be small enough to run out, or the fixed order
        // never touches the taxable account and there is nothing to compare.
        let portfolio = vec![
            account("ISA", FREE, "50000", "0", "0"),
            account("Pension", INCOME, "200000", "0", "0"),
        ];
        let run = |strategy| {
            calculate(&strategy_run(
                portfolio.clone(),
                "1",
                "60",
                "2000",
                strategy,
                Some(taxed("0", "60")),
            ))
            .unwrap()
        };
        let greedy = run(Strategy::CheapestFirst);
        let fixed = run(Strategy::Ordered { order: vec![FREE.into(), INCOME.into()] });
        assert!(
            greedy.tax_paid_total < fixed.tax_paid_total,
            "the greedy should claim allowances the fixed order wastes: {} vs {}",
            greedy.tax_paid_total,
            fixed.tax_paid_total
        );
        assert!(
            greedy.unused_allowance_total < fixed.unused_allowance_total,
            "and that is exactly what the unused-allowance figure should show"
        );
    }

    #[test]
    fn ordered_empties_each_account_kind_in_turn() {
        let input = strategy_run(
            mixed_portfolio(),
            "1",
            "120",
            "2000",
            Strategy::Ordered { order: vec![GAINS.into(), FREE.into(), INCOME.into()] },
            Some(taxed("0", "60")),
        );
        let out = calculate(&input).unwrap();
        let depleted = |name: &str| {
            out.investments
                .iter()
                .find(|r| r.name == name)
                .and_then(|r| r.depletion_month)
        };
        let unwrapped = depleted("Unwrapped").expect("the first account should empty");
        let cash = depleted("Cash").expect("the second account should empty");
        assert!(unwrapped < cash, "the order must be honoured: {unwrapped} then {cash}");
        assert!(
            depleted("Pension").is_none_or(|p| p > cash),
            "the pension is spent last"
        );
    }

    #[test]
    fn an_account_kind_missing_from_the_order_is_appended_not_rejected() {
        // A forgiving rule on purpose: a partial order should still project.
        let input = strategy_run(
            mixed_portfolio(),
            "1",
            "60",
            "1000",
            Strategy::Ordered { order: vec![GAINS.into()] },
            Some(taxed("0", "60")),
        );
        let out = calculate(&input).expect("a partial order must still project");
        assert!(out.withdrawn_total > Decimal::ZERO);
    }

    #[test]
    fn preserve_growth_needs_no_tax_system_at_all() {
        // The one strategy defined purely over returns, so it must work on a
        // projection that has never heard of tax.
        let input = strategy_run(
            vec![holding("Slow", "10000", "0", "0"), holding("Fast", "10000", "10", "0")],
            "1",
            "24",
            "500",
            Strategy::PreserveGrowth,
            None,
        );
        let out = calculate(&input).expect("PreserveGrowth must be legal untaxed");
        assert_eq!(out.tax_paid_total, Decimal::ZERO);
        assert_eq!(out.net_withdrawn_total, out.withdrawn_total);
    }

    #[test]
    fn preserve_growth_leaves_more_behind_than_splitting_pro_rata() {
        // Draining the worst compounder first leaves the best one compounding,
        // which is the whole claim of the strategy.
        let portfolio =
            vec![holding("Slow", "10000", "0", "0"), holding("Fast", "10000", "10", "0")];
        let preserve = calculate(&strategy_run(
            portfolio.clone(),
            "1",
            "120",
            "100",
            Strategy::PreserveGrowth,
            None,
        ))
        .unwrap();
        let pro_rata =
            calculate(&strategy_run(portfolio, "1", "120", "100", Strategy::ProRata, None)).unwrap();
        assert!(
            preserve.projected_total > pro_rata.projected_total,
            "preserving the compounder should end richer: {} vs {}",
            preserve.projected_total,
            pro_rata.projected_total
        );
    }

    #[test]
    fn preserve_growth_drains_the_worst_compounder_first() {
        let input = strategy_run(
            vec![holding("Slow", "10000", "0", "0"), holding("Fast", "10000", "10", "0")],
            "1",
            "240",
            "150",
            Strategy::PreserveGrowth,
            None,
        );
        let out = calculate(&input).unwrap();
        let slow = out.investments[0].depletion_month.expect("the 0% holding empties");
        assert!(
            out.investments[1].depletion_month.is_none_or(|f| f > slow),
            "the better compounder must outlast the worse one"
        );
    }

    #[test]
    fn a_rate_cap_is_honoured_while_it_can_be() {
        // Enough tax-free money about that the cap never has to bind.
        let input = strategy_run(
            vec![
                account("ISA", FREE, "200000", "0", "0"),
                account("Pension", INCOME, "50000", "0", "0"),
            ],
            "1",
            "24",
            "1000",
            Strategy::RateCapped { max_rate: "0" .into() },
            Some(taxed("50000", "60")),
        );
        let out = calculate(&input).unwrap();
        assert!(!out.rate_cap_breached, "there was tax-free money to take instead");
        assert_eq!(out.tax_paid_total, Decimal::ZERO, "a 0% cap must cost nothing");
    }

    #[test]
    fn a_rate_cap_that_cannot_be_met_is_breached_and_reported() {
        // Only a taxable account, other income already past the free band, and a
        // 0% cap. The money must still arrive, and the breach must be visible --
        // silently delivering less than asked would be the worse failure.
        let input = strategy_run(
            vec![account("Pension", INCOME, "200000", "0", "0")],
            "1",
            "24",
            "1000",
            Strategy::RateCapped { max_rate: "0".into() },
            Some(taxed("50000", "60")),
        );
        let out = calculate(&input).unwrap();
        assert!(out.rate_cap_breached, "the cap could not be honoured, so say so");
        assert!(out.tax_paid_total > Decimal::ZERO);
        assert_eq!(
            out.net_withdrawn_total.round_dp(2),
            Decimal::from(24_000),
            "the requested income is still delivered in full"
        );
    }

    #[test]
    fn a_tax_aware_order_without_tax_details_is_refused_at_the_control() {
        let input =
            strategy_run(mixed_portfolio(), "1", "24", "1000", Strategy::CheapestFirst, None);
        let err = calculate(&input).unwrap_err();
        assert_eq!(err.field, Some(Field::Strategy), "the message must name a control");
    }

    #[test]
    fn an_unknown_account_kind_names_the_row_that_carries_it() {
        let input = strategy_run(
            vec![account("Mystery", "not_a_real_account", "1000", "0", "0")],
            "1",
            "24",
            "10",
            Strategy::CheapestFirst,
            Some(taxed("0", "60")),
        );
        let err = calculate(&input).unwrap_err();
        assert_eq!(
            err.field,
            Some(Field::Investment { index: 0, part: InvestmentField::AccountKind })
        );
    }

    #[test]
    fn a_bad_cost_basis_names_the_row_but_only_where_it_is_consulted() {
        let bad = strategy_run(
            vec![account("Unwrapped", GAINS, "1000", "0", "nonsense")],
            "1",
            "24",
            "10",
            Strategy::CheapestFirst,
            Some(taxed("0", "60")),
        );
        assert_eq!(
            calculate(&bad).unwrap_err().field,
            Some(Field::Investment { index: 0, part: InvestmentField::CostBasis })
        );

        // The same nonsense on an account that never looks at a cost basis is
        // stale form state, not an error.
        let ignored = strategy_run(
            vec![account("Cash", FREE, "1000", "0", "nonsense")],
            "1",
            "24",
            "10",
            Strategy::CheapestFirst,
            Some(taxed("0", "60")),
        );
        assert!(calculate(&ignored).is_ok(), "an unused field must not block the projection");
    }

    #[test]
    fn a_pension_below_the_access_age_reports_against_the_age_control() {
        let input = strategy_run(
            vec![account("Pension", INCOME, "100000", "0", "0")],
            "1",
            "24",
            "1000",
            Strategy::CheapestFirst,
            Some(taxed("0", "40")),
        );
        let err = calculate(&input).unwrap_err();
        assert_eq!(err.field, Some(Field::Age));
        // The sentence itself is the tax system's, so `calc` never hard-codes
        // an age or an account name.
        assert!(err.message.contains("55"), "got: {}", err.message);
    }

    #[test]
    fn portfolio_depletion_is_the_last_holding_to_empty() {
        let input = strategy_run(
            mixed_portfolio(),
            "1",
            "600",
            "3000",
            Strategy::Ordered { order: vec![GAINS.into(), FREE.into(), INCOME.into()] },
            Some(taxed("0", "60")),
        );
        let out = calculate(&input).unwrap();
        let portfolio = out.depletion_month.expect("this draw exhausts the pot");
        let last_row = out
            .investments
            .iter()
            .filter_map(|r| r.depletion_month)
            .max()
            .expect("every holding empties");
        assert_eq!(portfolio, last_row);
        assert!(
            out.investments.iter().all(|r| r.depletion_month.is_some()),
            "everything is spent by the end"
        );
    }

    #[test]
    fn a_holding_built_from_deposits_still_reports_when_it_runs_out() {
        // Its value *today* is nothing -- the whole holding is deposits. That
        // says nothing about whether it later had something to run out of, so
        // gating depletion on today's value would leave this row silent.
        let mut built = account("Built up", FREE, "0", "0", "");
        built.contribution = "1000".into();
        let input = strategy_run(
            vec![built],
            "12",
            "24",
            "500",
            Strategy::ProRata,
            None,
        );
        let out = calculate(&input).unwrap();
        assert!(
            out.investments[0].current_value.is_zero(),
            "the premise: nothing there today"
        );
let row = out.investments[0]
            .depletion_month
            .expect("12,000 of deposits drawn at 500 a month must run out inside 24");
        assert_eq!(
            Some(row),
            out.depletion_month,
            "the only holding empties exactly when the portfolio does"
        );
    }

    #[test]
    fn cost_basis_defaults_to_todays_value_so_only_future_growth_is_taxed() {
        // A blank cost basis means "nothing gained yet". Selling immediately at
        // 0% growth should therefore cost nothing, where a zero basis would tax
        // the whole disposal.
        let blank = strategy_run(
            vec![account("Unwrapped", GAINS, "100000", "0", "")],
            "1",
            "12",
            "2000",
            Strategy::CheapestFirst,
            Some(taxed("0", "60")),
        );
        assert_eq!(calculate(&blank).unwrap().tax_paid_total, Decimal::ZERO);

        let all_gain = strategy_run(
            vec![account("Unwrapped", GAINS, "100000", "0", "0")],
            "1",
            "12",
            "2000",
            Strategy::CheapestFirst,
            Some(taxed("0", "60")),
        );
        assert!(
            calculate(&all_gain).unwrap().tax_paid_total > Decimal::ZERO,
            "a holding that is all profit is taxable"
        );
    }

    #[test]
    fn accounts_touched_counts_the_accounts_actually_used_each_period() {
        // The simplicity axis. One account can cover this draw, so the answer
        // should be one -- not "three, because there are three holdings".
        let input = strategy_run(
            mixed_portfolio(),
            "1",
            "12",
            "500",
            Strategy::Ordered { order: vec![GAINS.into(), FREE.into(), INCOME.into()] },
            Some(taxed("0", "60")),
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.accounts_touched, vec![1]);
    }

    // --- solve: monthly top-up ---------------------------------------------

    #[test]
    fn top_up_solves_a_hand_checkable_case() {
        let input = with_contribution("0", "0", "0", "120", Unit::Months);
        let sol = solve(&input, &Goal::MonthlyTopUp { target: "12000".into() }).unwrap();
        assert_eq!(sol, Solution::MonthlyTopUp(d("100.00")));
    }

    #[test]
    fn top_up_answer_round_trips_and_a_penny_less_falls_short() {
        let input = with_contribution("5000", "6", "0", "15", Unit::Years);
        let target = d("250000");
        let Solution::MonthlyTopUp(top_up) =
            solve(&input, &Goal::MonthlyTopUp { target: "250000".into() }).unwrap()
        else {
            panic!("expected a MonthlyTopUp solution");
        };

        let reached = |c: Decimal| {
            let mut probe = input.clone();
            probe.investments[0].contribution = c.to_string();
            calculate(&probe).unwrap().projected_total
        };
        assert!(reached(top_up) >= target, "reported top-up must reach the target");
        assert!(reached(top_up - d("0.01")) < target, "a penny less must fall short");
    }

    #[test]
    fn top_up_reports_already_met_when_no_contribution_is_needed() {
        let input = with_contribution("100000", "7", "0", "10", Unit::Years);
        let sol = solve(&input, &Goal::MonthlyTopUp { target: "150000".into() }).unwrap();
        assert_eq!(sol, Solution::AlreadyMet);
    }

    #[test]
    fn top_up_target_out_of_range_errors_with_a_message() {
        let input = with_contribution("1", "0", "0", "1", Unit::Months);
        let err = solve(&input, &Goal::MonthlyTopUp { target: "999999999999".into() }).unwrap_err();
        assert!(err.message.contains("No monthly top-up reaches"));
        assert!(err.field.is_none());
    }

    #[test]
    fn top_up_rejects_a_bad_target() {
        let input = with_contribution("1000", "7", "0", "10", Unit::Years);
        assert!(solve(&input, &Goal::MonthlyTopUp { target: "abc".into() }).is_err());
        assert!(solve(&input, &Goal::MonthlyTopUp { target: "-5".into() }).is_err());
    }

    #[test]
    fn top_up_splits_a_portfolio_target_across_holdings() {
        // £501,000 today across two holdings at 0%, 120 months. Reaching £513,000
        // needs £12,000 more = £100/month total, split across the two.
        let input = deposits(
            vec![holding("Small", "1000", "0", "0"), holding("Large", "500000", "0", "0")],
            "120",
            Unit::Months,
        );
        let sol = solve(&input, &Goal::MonthlyTopUp { target: "513000".into() }).unwrap();
        assert_eq!(sol, Solution::MonthlyTopUp(d("100.00")));
    }

    // --- solve: time to target ---------------------------------------------

    #[test]
    fn time_to_target_on_a_flat_contribution_case() {
        let input = with_contribution("0", "0", "100", "10", Unit::Years);
        let sol = solve(&input, &Goal::TimeToTarget { target: "1200".into() }).unwrap();
        assert_eq!(sol, Solution::Months(12));
    }

    #[test]
    fn time_to_target_reports_already_met_when_value_today_clears_it() {
        let input = with_contribution("50000", "7", "0", "10", Unit::Years);
        let sol = solve(&input, &Goal::TimeToTarget { target: "40000".into() }).unwrap();
        assert_eq!(sol, Solution::AlreadyMet);
    }

    #[test]
    fn time_to_target_that_is_never_reached_errors_not_hangs() {
        let input = with_contribution("1000", "0", "0", "10", Unit::Years);
        let err = solve(&input, &Goal::TimeToTarget { target: "5000".into() }).unwrap_err();
        assert!(err.message.contains("does not reach"));
    }

    #[test]
    fn time_to_target_ignores_a_drawdown_plan() {
        // Asked of a drawdown input, a deposits goal answers the accumulation
        // question — it does not draw the pot down.
        let dep = with_contribution("0", "0", "100", "10", Unit::Years);
        let dd = drawdown(vec![holding("X", "0", "0", "100")], "10", Unit::Years, "30", Unit::Years, "5000");
        assert_eq!(
            solve(&dep, &Goal::TimeToTarget { target: "1200".into() }).unwrap(),
            solve(&dd, &Goal::TimeToTarget { target: "1200".into() }).unwrap()
        );
    }

    #[test]
    fn a_portfolio_goal_needs_a_holding() {
        let empty = deposits(vec![], "10", Unit::Years);
        assert!(solve(&empty, &Goal::TimeToTarget { target: "1000".into() }).is_err());
        assert!(solve(&empty, &Goal::MonthlyTopUp { target: "1000".into() }).is_err());
    }

    // --- solve: maximum sustainable withdrawal -----------------------------

    #[test]
    fn max_withdrawal_empties_the_pot_at_the_end_of_the_drawdown() {
        // Grow £100k at 5% for nothing (0 grow months would be invalid), then draw
        // it down over 30 years. The reported draw must survive the period and a
        // penny more must not.
        let input = drawdown(vec![holding("X", "100000", "5", "0")], "1", Unit::Months, "30", Unit::Years, "0");
        let Solution::MaxWithdrawal(w) = solve(&input, &Goal::MaxWithdrawal).unwrap() else {
            panic!("expected a MaxWithdrawal solution");
        };
        assert!(w > Decimal::ZERO);

        let round_trip = |draw: &str| {
            let probe = drawdown(vec![holding("X", "100000", "5", "0")], "1", Unit::Months, "30", Unit::Years, draw);
            calculate(&probe).unwrap()
        };
        assert_eq!(round_trip(&w.to_string()).depletion_month, None, "must last the period");
        assert!(round_trip(&(w + d("0.01")).to_string()).depletion_month.is_some(), "a penny more depletes early");
    }

    #[test]
    fn max_withdrawal_matches_the_single_holding_annuity() {
        // One holding, no deposits, drawn down over D months at monthly factor f.
        // The exact sustainable draw is the annuity payment P*(f-1)/(1 - f^-D).
        // The solver (which clamps at £0 and rounds down) must be within a penny.
        let grow = 12u32; // 1 year of growth
        let draw_years = 20u32;
        let d_months = draw_years * 12;
        let input = drawdown(
            vec![holding("X", "500000", "4", "0")],
            &grow.to_string(),
            Unit::Months,
            &draw_years.to_string(),
            Unit::Years,
            "0",
        );
        let Solution::MaxWithdrawal(w) = solve(&input, &Goal::MaxWithdrawal).unwrap() else {
            panic!("expected a MaxWithdrawal solution");
        };

        // Pot at the start of drawdown, and the monthly factor.
        let base = calculate(&input).unwrap();
        let pot = base.handover_total.unwrap();
        let f = (Decimal::ONE + d("0.04")).powd(Decimal::ONE / Decimal::from(12u32));
        let f_pow = f.powd(Decimal::from(d_months));
        let annuity = pot * (f - Decimal::ONE) / (Decimal::ONE - Decimal::ONE / f_pow);
        assert!((w - annuity).abs() < d("1"), "solver {w} vs annuity {annuity}");
    }

    #[test]
    fn max_withdrawal_needs_a_drawdown_plan() {
        let input = with_contribution("100000", "5", "0", "10", Unit::Years);
        assert!(solve(&input, &Goal::MaxWithdrawal).is_err());
    }

    // --- solve: time to deplete --------------------------------------------

    #[test]
    fn time_to_deplete_agrees_with_the_projection() {
        // £12,000 pot at 0% (grown 1 month, flat), drawing £500 a month runs dry 24
        // months into the drawdown. The absolute depletion month is grow + 24.
        let input = drawdown(vec![holding("X", "12000", "0", "0")], "1", Unit::Months, "1", Unit::Months, "500");
        let sol = solve(&input, &Goal::TimeToDeplete).unwrap();
        assert_eq!(sol, Solution::Depletes(24));

        // Cross-check against calculate over a long-enough period.
        let long = drawdown(vec![holding("X", "12000", "0", "0")], "1", Unit::Months, "60", Unit::Months, "500");
        assert_eq!(calculate(&long).unwrap().depletion_month, Some(1 + 24));
    }

    #[test]
    fn a_draw_covered_by_returns_never_depletes() {
        // £100,000 at 6% earns ~£490 in month one, so a £100 draw is covered.
        let input = drawdown(vec![holding("X", "100000", "6", "0")], "1", Unit::Months, "30", Unit::Years, "100");
        assert_eq!(solve(&input, &Goal::TimeToDeplete).unwrap(), Solution::NeverDepletes);
        // Drawing nothing is trivially never.
        let flat = drawdown(vec![holding("X", "100000", "6", "0")], "1", Unit::Months, "30", Unit::Years, "0");
        assert_eq!(solve(&flat, &Goal::TimeToDeplete).unwrap(), Solution::NeverDepletes);
    }

    #[test]
    fn a_larger_draw_never_lasts_longer() {
        // Monotonicity is the invariant the whole drawdown search rests on.
        let span = |amount: &str| {
            let input = drawdown(vec![holding("X", "50000", "4", "0")], "1", Unit::Months, "100", Unit::Years, amount);
            match solve(&input, &Goal::TimeToDeplete).unwrap() {
                Solution::Depletes(m) => m,
                Solution::NeverDepletes => u32::MAX,
                other => panic!("unexpected solution {other:?}"),
            }
        };
        let mut previous = u32::MAX;
        for amount in ["100", "200", "400", "800", "1600", "3200", "6400"] {
            let months = span(amount);
            assert!(months <= previous, "drawing {amount} lasted {months}, longer than {previous}");
            previous = months;
        }
    }

    #[test]
    fn time_to_deplete_splits_the_draw_across_holdings() {
        // Two holdings at 0%, £2,000 a month. Under monthly pro-rata the whole
        // £501,000 empties together at month 251 of drawdown (501000 / 2000).
        let input = drawdown(
            vec![holding("Small", "1000", "0", "0"), holding("Large", "500000", "0", "0")],
            "1",
            Unit::Months,
            "100",
            Unit::Years,
            "2000",
        );
        // 501000 / 2000 = 250.5, so month 251 empties it.
        assert_eq!(solve(&input, &Goal::TimeToDeplete).unwrap(), Solution::Depletes(251));
    }

    #[test]
    fn time_to_deplete_needs_a_drawdown_plan() {
        let input = with_contribution("100000", "5", "0", "10", Unit::Years);
        assert!(solve(&input, &Goal::TimeToDeplete).is_err());
    }
}

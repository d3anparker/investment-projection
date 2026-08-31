//! Withdrawal strategies: the two orthogonal axes of a drawdown.
//!
//! An [`Order`] (how the monthly withdrawal is apportioned) paired with a stop
//! [`Limit`], bundled as a [`Strategy`] with named constructors for the
//! combinations the UI offers. Pure vocabulary — no engine or tax types.

/// How a monthly withdrawal is apportioned across the holdings — the *ordering*
/// axis of a drawdown [`Strategy`].
///
/// Ordering is expressed over **opaque account-kind ids**, never over a named
/// enum of wrappers: `calc` carries the ids a [`TaxSystem`] advertises and never
/// learns what any of them mean.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Order {
    /// Split the withdrawal across every holding pro-rata by current value,
    /// rebalanced monthly, ignoring tax entirely. The original behaviour, and
    /// the default: an input that says nothing about tax projects exactly as it
    /// did before the tax model existed, and this is the one order taken *gross*.
    #[default]
    ProRata,
    /// Empty each account kind in turn, in the order given, splitting pro-rata
    /// within a kind. Kinds present in the portfolio but missing from `order`
    /// are appended by their catalogue rank rather than treated as an error.
    ByKind(Vec<String>),
    /// Drain the lowest-returning holding first, so the best compounder is left
    /// to compound. A **non-tax** objective: it needs no [`TaxContext`] at all
    /// and is legal on an untaxed projection.
    ByReturn,
    /// Each month, take from whichever holding keeps most of the next pound —
    /// a dynamic, re-ranked-every-rate-boundary order rather than a fixed one.
    /// The only order whose month is a greedy argmax rather than a static split.
    ByMarginalCost,
}

/// When to stop drawing from a holding — the *stop-rule* axis of a [`Strategy`],
/// orthogonal to its [`Order`].
///
/// For the static orders the answer is always [`Limit::Requirement`]; the rate
/// boundaries only bite under [`Order::ByMarginalCost`], where a rung stop lets
/// the order be reconsidered and a rate cap keeps the draw out of a band.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Limit {
    /// Take as much as it takes to meet the requirement, or empty the holding.
    #[default]
    Requirement,
    /// Stop as soon as the marginal rate would step up, so a dynamic order can
    /// reconsider which holding is now cheapest.
    NextRung,
    /// Draw from a holding only while its marginal rate is at or below the cap
    /// (a percent string), then move on. If every holding is capped out and the
    /// requirement is still unmet, the shortfall is drawn anyway and
    /// [`CalcOutput::rate_cap_breached`] is set — delivering the money and
    /// saying so beats silently short-changing the withdrawal.
    RateCap(String),
}

/// How a monthly withdrawal is taken: an [`Order`] paired with a stop [`Limit`].
///
/// The two axes are orthogonal — the earlier flat enum sparsely populated the
/// grid and made "cap the rate but spend in the conventional order" inexpressible.
/// The five named constructors below are the combinations the UI offers; other
/// pairings are legal to construct but only [`Order::ByMarginalCost`] consults
/// the stop (a static order always draws to [`Limit::Requirement`]).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Strategy {
    pub order: Order,
    pub stop: Limit,
}

impl Strategy {
    /// Split pro-rata across every holding, gross. The default.
    pub fn pro_rata() -> Self {
        Strategy { order: Order::ProRata, stop: Limit::Requirement }
    }
    /// Empty each account kind in turn, in the given order.
    pub fn ordered(order: Vec<String>) -> Self {
        Strategy { order: Order::ByKind(order), stop: Limit::Requirement }
    }
    /// Take from the cheapest holding each month, re-ranking at every rate rung.
    pub fn cheapest_first() -> Self {
        Strategy { order: Order::ByMarginalCost, stop: Limit::NextRung }
    }
    /// Drain the lowest-returning holding first.
    pub fn preserve_growth() -> Self {
        Strategy { order: Order::ByReturn, stop: Limit::Requirement }
    }
    /// Take from the cheapest holding, but never above `max_rate` at the margin.
    pub fn rate_capped(max_rate: String) -> Self {
        Strategy { order: Order::ByMarginalCost, stop: Limit::RateCap(max_rate) }
    }

    /// Whether this strategy needs to know what a withdrawal costs — a tax-
    /// motivated order, or a stop rule expressed in terms of a tax rate.
    pub(crate) fn needs_tax(&self) -> bool {
        matches!(self.order, Order::ByKind(_) | Order::ByMarginalCost)
            || matches!(self.stop, Limit::NextRung | Limit::RateCap(_))
    }

    /// Whether the plan's `withdrawal` is a net figure. Only [`Order::ProRata`]
    /// takes it gross.
    pub fn withdrawal_is_net(&self) -> bool {
        self.order != Order::ProRata
    }
}

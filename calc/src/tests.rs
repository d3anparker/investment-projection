//! Core unit tests. See the crate root and CLAUDE.md for the invariants
//! these pin (deposits stop at handover, gross == net + tax, residue rule…).

use crate::*;
use rust_decimal::Decimal;
use std::str::FromStr;
use rust_decimal::MathematicalOps;
use crate::types::NEUTRAL_CURRENCY;
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
            currency: String::new(),
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
                strategy: Strategy::pro_rata(),
            },
            currency: String::new(),
            tax: None,
        }
    }

    /// A tax context over the mock system: a fictional jurisdiction with round
    /// numbers, so these tests pin the *engine* rather than one country's
    /// figures. A rate change in `uk-tax` must not be able to break `calc`.
    fn taxed(other_income: &str, age: &str) -> TaxContext {
        TaxContext {
            system: &taxkit::mock::MOCK,
            region: "all".into(),
            other_income: other_income.into(),
            age: age.into(),
            uprate: "0".into(),
            options: Vec::new(),
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
            currency: String::new(),
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
                currency: String::new(),
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
    // in `uk-tax` next April cannot break `calc`, and it is the second
    // implementation that proves nothing jurisdiction-specific leaked in here.

    use taxkit::mock::{FREE, GAINS, INCOME};

    /// Every strategy over the same portfolio, for the invariants that must hold
    /// under all of them.
    fn every_strategy() -> Vec<(&'static str, Strategy)> {
        vec![
            ("pro-rata", Strategy::pro_rata()),
            ("ordered", Strategy::ordered(vec![GAINS.into(), FREE.into(), INCOME.into()])),
            ("cheapest first", Strategy::cheapest_first()),
            ("preserve growth", Strategy::preserve_growth()),
            ("rate capped", Strategy::rate_capped("20".into())),
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
        let untaxed = strategy_run(mixed_portfolio(), "12", "60", "2000", Strategy::pro_rata(), None);
        let taxed_input = strategy_run(
            mixed_portfolio(),
            "12",
            "60",
            "2000",
            Strategy::pro_rata(),
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

            // Net is not carried as its own series — a caller derives it as
            // `gross - tax`. What the series must guarantee is that doing so
            // never goes negative: tax never exceeds the gross it is charged on,
            // pointwise on the rounded figures.
            for (i, (gross, tax)) in out
                .withdrawals_series
                .iter()
                .zip(out.tax_series.iter())
                .enumerate()
            {
                assert!(*tax <= *gross, "{name}: month {i} tax cannot exceed gross");
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
            if matches!(strategy.order, Order::ProRata) {
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
            Strategy::cheapest_first(),
            Some(taxed("0", "60")),
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.tax_paid_total, Decimal::ZERO, "each year's allowance covers it");
        assert_eq!(out.withdrawn_total, Decimal::from(24_000));
        // The 24-month drawdown spans two twelve-month periods; the allowance
        // resets at the boundary, which is why the second year is free again.
        assert_eq!(out.period_months, Some(12));
        assert_eq!(out.accounts_touched.len(), 2, "two tax periods");
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
            Strategy::cheapest_first(),
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
        let greedy = run(Strategy::cheapest_first());
        let fixed = run(Strategy::ordered(vec![FREE.into(), INCOME.into()]));
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
            Strategy::ordered(vec![GAINS.into(), FREE.into(), INCOME.into()]),
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
            Strategy::ordered(vec![GAINS.into()]),
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
            Strategy::preserve_growth(),
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
            Strategy::preserve_growth(),
            None,
        ))
        .unwrap();
        let pro_rata =
            calculate(&strategy_run(portfolio, "1", "120", "100", Strategy::pro_rata(), None)).unwrap();
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
            Strategy::preserve_growth(),
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
            Strategy::rate_capped("0".into()),
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
            Strategy::rate_capped("0".into()),
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
            strategy_run(mixed_portfolio(), "1", "24", "1000", Strategy::cheapest_first(), None);
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
            Strategy::cheapest_first(),
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
            Strategy::cheapest_first(),
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
            Strategy::cheapest_first(),
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
            Strategy::cheapest_first(),
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
            Strategy::ordered(vec![GAINS.into(), FREE.into(), INCOME.into()]),
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
            Strategy::pro_rata(),
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
            Strategy::cheapest_first(),
            Some(taxed("0", "60")),
        );
        assert_eq!(calculate(&blank).unwrap().tax_paid_total, Decimal::ZERO);

        let all_gain = strategy_run(
            vec![account("Unwrapped", GAINS, "100000", "0", "0")],
            "1",
            "12",
            "2000",
            Strategy::cheapest_first(),
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
            Strategy::ordered(vec![GAINS.into(), FREE.into(), INCOME.into()]),
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
    fn money_in_messages_uses_the_supplied_currency_symbol() {
        // `calc` names no currency: with none supplied the message carries the
        // neutral marker, and the caller's symbol is used verbatim when given.
        let mut input = with_contribution("1", "0", "0", "1", Unit::Months);
        let goal = Goal::MonthlyTopUp { target: "999999999999".into() };

        let neutral = solve(&input, &goal).unwrap_err();
        assert!(neutral.message.contains(NEUTRAL_CURRENCY));
        assert!(!neutral.message.contains('\u{00a3}'));

        input.currency = "\u{00a3}".into();
        let pounds = solve(&input, &goal).unwrap_err();
        assert!(pounds.message.contains('\u{00a3}'));
        assert!(!pounds.message.contains(NEUTRAL_CURRENCY));
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

// --- MOCK_LEVY: the periodic-charge and options machinery ------------------
//
// Pinned against the fictional levying system, not real German figures, so a
// Vorabpauschale rate change can no more break `calc` than an April UK change
// can. These are the invariants a *charging* tax system adds.
mod levy {
    use super::*;
    use taxkit::mock::{FUND, MOCK_LEVY, PLAIN};

    fn levy_ctx(options: Vec<(String, String)>) -> TaxContext {
        TaxContext {
            system: &MOCK_LEVY,
            region: "all".into(),
            other_income: "0".into(),
            age: "60".into(),
            uprate: "0".into(),
            options,
        }
    }

    fn opt(id: &str, value: &str) -> (String, String) {
        (id.into(), value.into())
    }

    /// Accumulation-only, but taxed — the charge fires while accumulating, which
    /// `strategy_run` (always a drawdown) cannot express.
    fn deposits_taxed(
        investments: Vec<InvestmentInput>,
        horizon: &str,
        tax: TaxContext,
    ) -> CalcInput {
        CalcInput {
            investments,
            horizon_value: horizon.into(),
            horizon_unit: Unit::Months,
            plan: Plan::Deposits,
            currency: String::new(),
            tax: Some(tax),
        }
    }

    #[test]
    fn the_levy_lands_on_funds_and_sums_to_charged_total() {
        // Two 100,000 holdings, 0% return, 24 months of accumulation. The one
        // period boundary the loop reaches (month 12, charging the year [0,12))
        // levies 2% of the fund's 100,000 opening = 2,000 taxable, less the
        // 1,000 allowance, at 20% = 200. The plain account pays nothing.
        let input = deposits_taxed(
            vec![
                account("Cash", PLAIN, "100000", "0", "0"),
                account("Fund", FUND, "100000", "0", "0"),
            ],
            "24",
            levy_ctx(vec![]),
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.charged_total, d("200.00"));
        assert_eq!(out.investments[0].charged, d("0.00"), "plain account is not levied");
        assert_eq!(out.investments[1].charged, d("200.00"), "fund carries the whole charge");
        // The charge came out of the pot.
        assert_eq!(out.projected_total, d("199800.00"));
    }

    #[test]
    fn zero_return_growth_is_zero_despite_the_levy_and_withdrawals() {
        // The reconciliation test: at 0% return there is no investment gain, so
        // `growth` must be exactly zero even though the levy and the withdrawals
        // both move money out — each is added back in the growth identity.
        let input = strategy_run(
            vec![account("Fund", FUND, "1000000", "0", "0")],
            "12",
            "24",
            "500",
            Strategy::pro_rata(),
            Some(levy_ctx(vec![])),
        );
        let out = calculate(&input).unwrap();
        assert!(out.charged_total > d("0.00"), "the levy must actually fire");
        assert_eq!(out.growth, d("0.00"), "a 0% return is zero growth, charge notwithstanding");
        // The full identity, recomputed independently of `growth`.
        assert_eq!(
            out.projected_total,
            out.current_total + out.contributed_total - out.withdrawn_total - out.charged_total
                + out.growth,
        );
    }

    #[test]
    fn pro_rata_withdrawals_are_unchanged_by_the_levy() {
        // Restated invariant (9): a periodic charge means pro-rata is no longer
        // byte-identical to untaxed — the pot is smaller. But its *withdrawals*
        // must be, because they never route through the session. A big pot and a
        // small draw so nothing depletes in either run.
        let portfolio = || vec![account("Fund", FUND, "1000000", "0", "0")];
        let untaxed =
            strategy_run(portfolio(), "12", "12", "500", Strategy::pro_rata(), None);
        let levied = strategy_run(
            portfolio(),
            "12",
            "12",
            "500",
            Strategy::pro_rata(),
            Some(levy_ctx(vec![])),
        );
        let u = calculate(&untaxed).unwrap();
        let l = calculate(&levied).unwrap();
        assert_eq!(u.withdrawn_total, l.withdrawn_total, "withdrawal total unchanged");
        assert_eq!(
            u.investments[0].withdrawn, l.investments[0].withdrawn,
            "per-row withdrawal unchanged"
        );
        assert_eq!(u.charged_total, d("0.00"), "no charge without a charging system");
        assert!(l.charged_total > d("0.00"), "the levy fired");
        assert!(l.projected_total < u.projected_total, "the levy shrank the pot");
    }

    #[test]
    fn the_joint_option_doubles_the_allowance() {
        // 24 months, one 100,000 fund. Single: 2,000 base − 1,000 allowance at
        // 20% = 200. Joint doubles the allowance to 2,000, which covers the whole
        // base, so nothing is charged.
        let run = |options: Vec<(String, String)>| {
            let input = deposits_taxed(
                vec![account("Fund", FUND, "100000", "0", "0")],
                "24",
                levy_ctx(options),
            );
            calculate(&input).unwrap().charged_total
        };
        assert_eq!(run(vec![]), d("200.00"), "single assessment");
        assert_eq!(run(vec![opt("joint", "true")]), d("0.00"), "joint covers the base");
    }

    #[test]
    fn the_cohort_option_lowers_withdrawal_tax() {
        // The cohort year halves the taxable fraction of a fund withdrawal from
        // 2030 on. An ordered strategy (so withdrawals are actually taxed) drawn
        // over two years; the later cohort pays strictly less tax.
        let run = |cohort: &str| {
            let input = strategy_run(
                vec![account("Fund", FUND, "500000", "0", "0")],
                "12",
                "24",
                "3000",
                Strategy::cheapest_first(),
                Some(levy_ctx(vec![opt("cohort", cohort)])),
            );
            calculate(&input).unwrap().tax_paid_total
        };
        let old_cohort = run("2020");
        let new_cohort = run("2035");
        assert!(old_cohort > d("0.00"), "the old cohort is taxed");
        assert!(
            new_cohort < old_cohort,
            "the post-2030 cohort's half-taxable withdrawals cost less: {new_cohort} vs {old_cohort}",
        );
    }

    #[test]
    fn gross_is_still_net_plus_tax_under_a_levy() {
        // The withdrawal identity holds pointwise even with a charge in play: the
        // charge is a separate flow and never muddles gross/net/tax.
        let input = strategy_run(
            vec![account("Fund", FUND, "500000", "0", "0")],
            "12",
            "36",
            "2000",
            Strategy::cheapest_first(),
            Some(levy_ctx(vec![])),
        );
        let out = calculate(&input).unwrap();
        assert_eq!(out.withdrawn_total, out.net_withdrawn_total + out.tax_paid_total);
        for (i, (gross, tax)) in out
            .withdrawals_series
            .iter()
            .zip(out.tax_series.iter())
            .enumerate()
        {
            assert!(*tax <= *gross, "month {i}: tax cannot exceed gross");
        }
    }
}

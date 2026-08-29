//! Display formatting: turning the `calc` core's `Decimal`s into the strings
//! shown in the UI. Pure and side-effect free.

use rust_decimal::Decimal;

/// Group an integer digit string into thousands, e.g. `"1234567" -> "1,234,567"`.
pub fn group_thousands(int_digits: &str) -> String {
    let len = int_digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in int_digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The currency to print amounts in, taken from the active tax system rather
/// than hard-coded — the last place the render path assumed a jurisdiction.
pub fn currency() -> &'static str {
    crate::convert::TAX_SYSTEM.currency_symbol()
}

/// Format a monetary amount as `1,234.56` behind the active currency symbol
/// (with a leading `-` when negative).
pub fn fmt_money(d: Decimal) -> String {
    let neg = d.is_sign_negative();
    let s = format!("{:.2}", d.abs());
    let (int, frac) = s.split_once('.').unwrap_or((s.as_str(), "00"));
    format!("{}{}{}.{}", if neg { "-" } else { "" }, currency(), group_thousands(int), frac)
}

/// Format a monetary amount with an explicit sign: `+£1,234.56` / `-£1,234.56`.
///
/// Used for gain/loss figures, where the direction must survive greyscale
/// printing and colour blindness. `fmt_money` shows a bare `£1,234.56` for a
/// positive, leaving green-vs-red as the only cue that it is a gain.
pub fn fmt_signed_money(d: Decimal) -> String {
    if d.is_zero() {
        return fmt_money(d);
    }
    if d.is_sign_negative() {
        // `fmt_money` already emits the minus sign.
        fmt_money(d)
    } else {
        format!("+{}", fmt_money(d))
    }
}

/// Format a growth *fraction* (e.g. `0.07`) as a signed percentage: `+7.00%`.
pub fn fmt_pct(fraction: Decimal) -> String {
    let p = (fraction * Decimal::from(100u32)).round_dp(2);
    let sign = if p.is_sign_positive() && !p.is_zero() { "+" } else { "" };
    format!("{}{:.2}%", sign, p)
}

/// Format a fraction as a plain percentage, with no sign.
///
/// [`fmt_pct`] exists to show *direction* (a gain or a loss), so it prepends a
/// `+`. A rate that has no direction -- a share of something, like tax as a
/// proportion of what was taken out -- must not borrow that `+`: "+18.42%" reads
/// as a rate that went up by 18.42%.
pub fn fmt_rate(fraction: Decimal) -> String {
    format!("{:.2}%", (fraction * Decimal::from(100u32)).round_dp(2))
}

/// Label one point on the projection timeline, for the chart's scrub readout:
/// `Today`, `Year 4`, `Month 7`. Distinct from [`horizon_label`], which names a
/// *duration* ("10 years") rather than a position on it.
pub fn month_label(months: u32) -> String {
    if months == 0 {
        "Today".to_string()
    } else if months % 12 == 0 {
        format!("Year {}", months / 12)
    } else {
        format!("Month {months}")
    }
}

/// Human-readable horizon label: whole years when divisible, else months.
pub fn horizon_label(months: u32) -> String {
    if months % 12 == 0 {
        let y = months / 12;
        format!("{} year{}", y, if y == 1 { "" } else { "s" })
    } else {
        format!("{} month{}", months, if months == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn month_label_names_a_position_not_a_duration() {
        assert_eq!(month_label(0), "Today");
        assert_eq!(month_label(12), "Year 1");
        assert_eq!(month_label(48), "Year 4");
        assert_eq!(month_label(7), "Month 7");
        // Contrast with horizon_label, which names the span.
        assert_eq!(horizon_label(12), "1 year");
    }

    #[test]
    fn signed_money_always_shows_direction() {
        // The sign is the non-colour cue for gain vs loss.
        assert_eq!(fmt_signed_money(d("1234.56")), "+\u{00a3}1,234.56");
        assert_eq!(fmt_signed_money(d("-1234.56")), "-\u{00a3}1,234.56");
        // Zero is neither, so it stays unsigned.
        assert_eq!(fmt_signed_money(d("0")), "\u{00a3}0.00");
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(group_thousands("1"), "1");
        assert_eq!(group_thousands("100"), "100");
        assert_eq!(group_thousands("1000"), "1,000");
        assert_eq!(group_thousands("1234567"), "1,234,567");
    }

    #[test]
    fn money_formats_sign_and_padding() {
        assert_eq!(fmt_money(d("0")), "\u{00a3}0.00");
        assert_eq!(fmt_money(d("1234.5")), "\u{00a3}1,234.50");
        assert_eq!(fmt_money(d("-89.9")), "-\u{00a3}89.90");
        // A whole number still shows two decimal places.
        assert_eq!(fmt_money(d("200")), "\u{00a3}200.00");
    }

    #[test]
    fn pct_signs_correctly() {
        assert_eq!(fmt_pct(d("0.07")), "+7.00%");
        assert_eq!(fmt_pct(d("0")), "0.00%");
        assert_eq!(fmt_pct(d("-0.105")), "-10.50%");
    }

    #[test]
    fn horizon_pluralises() {
        assert_eq!(horizon_label(12), "1 year");
        assert_eq!(horizon_label(120), "10 years");
        assert_eq!(horizon_label(1), "1 month");
        assert_eq!(horizon_label(18), "18 months");
    }
}

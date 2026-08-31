//! Lenient input parsing and display-string helpers.
//!
//! [`parse_number`] accepts numbers the way users type them; the rest format
//! money and periods for `calc`'s own error messages. Deliberately non-guessing.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::str::FromStr;

use crate::types::{CalcError, Unit};

/// A blank numeric field reads as zero, matching the rest of the form.
pub(crate) fn parse_or_zero(s: &str) -> Option<Decimal> {
    if s.trim().is_empty() {
        Some(Decimal::ZERO)
    } else {
        parse_number(s)
    }
}

/// A grouped `1,234.56` behind the caller's currency `symbol`, for embedding in
/// error messages, matching the UI's `fmt_money`. Kept here (not in the UI
/// `format` module) because `calc` owns its own message text; the symbol is
/// passed in rather than written because `calc` names no currency (see
/// `CalcInput::currency`).
pub(crate) fn fmt_money_plain(d: Decimal, symbol: &str) -> String {
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
    format!("{sign}{symbol}{grouped}.{frac}")
}

/// Convert a `value` + `unit` into whole months, doing the ×12 and the rounding
/// of fractional periods entirely in `Decimal`. `field` names the input for
/// error messages.
pub(crate) fn to_months(value: &str, unit: Unit, field: &str) -> Result<u32, String> {
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

pub(crate) fn round2(d: Decimal) -> Decimal {
    d.round_dp(2)
}

pub(crate) fn too_large_msg(name: &str) -> String {
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
pub(crate) fn overflowed() -> CalcError {
    CalcError::new(portfolio_too_large(), None)
}

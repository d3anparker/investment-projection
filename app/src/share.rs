//! The shareable-link codec that makes a projection linkable.
//!
//! The whole form state — every row, the periods, the mode and the goal — is
//! serialized to JSON, base64url-encoded, and carried in the URL *fragment* as
//! `#v=…` so a projection can be saved, revisited, or shared without a backend or
//! any storage. A fragment is never sent in the HTTP request, so this keeps the
//! "nothing leaves this page" property intact; the figures travel only inside a
//! link the user chooses to copy.
//!
//! Pure and natively tested. JSON comes from `serde_json` and the text encoding
//! from the `base64` crate — both pure Rust, so this module stays free of
//! `web_sys` and its round-trip can be tested off the browser. [`decode`] is
//! total and forgiving: any malformed input yields `None`, and the caller then
//! falls back to the built-in example exactly as a bare page load does.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};

use crate::convert::RowData;

/// The slice of `App` state a shared link carries.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ShareState {
    pub rows: Vec<RowData>,
    #[serde(default = "default_horizon_value")]
    pub horizon_value: String,
    #[serde(default = "default_horizon_unit")]
    pub horizon_unit: String,
    /// The top-level mode: `"deposits"` or `"drawdown"`.
    #[serde(default = "default_plan")]
    pub plan: String,
    #[serde(default = "default_drawdown_value")]
    pub drawdown_value: String,
    #[serde(default = "default_drawdown_unit")]
    pub drawdown_unit: String,
    /// The portfolio-level monthly withdrawal (drawdown mode).
    #[serde(default)]
    pub withdrawal: String,
    #[serde(default)]
    pub goal_target: String,
    #[serde(default = "default_goal_kind")]
    pub goal_kind: String,
    // The tax controls. Every one is `#[serde(default)]`, which is what lets
    // them be added *without* a `VERSION` bump: a link written before they
    // existed decodes with them blank, which is pro-rata and untaxed — exactly
    // the projection it used to show. Bumping instead would hard-reject every
    // link already in the wild, so prefer a defaulted field every time.
    /// The withdrawal-order picker. Blank is pro-rata.
    #[serde(default)]
    pub strategy: String,
    /// The rate cap belonging to the rate-capped strategy, as a percent.
    #[serde(default)]
    pub rate_cap: String,
    /// Which part of the jurisdiction the holder lives in. Blank is the tax
    /// system's first region.
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub other_income: String,
    #[serde(default)]
    pub age: String,
    /// Annual uprating of tax thresholds, as a percent. Blank freezes them.
    #[serde(default)]
    pub uprate: String,
}

fn default_horizon_value() -> String {
    "10".into()
}
fn default_horizon_unit() -> String {
    "years".into()
}
fn default_plan() -> String {
    "deposits".into()
}
fn default_drawdown_value() -> String {
    "30".into()
}
fn default_drawdown_unit() -> String {
    "years".into()
}
fn default_goal_kind() -> String {
    "topup".into()
}

impl ShareState {
    /// The built-in illustrative projection shown on a bare load — i.e. when the
    /// fragment holds no shared link (or a mangled one that [`decode`] rejects).
    /// A whole `ShareState` so the caller seeds every signal from one source
    /// instead of branching between a decoded link and inline defaults. The
    /// `RowData` ids are placeholders; the caller reassigns them.
    pub fn example() -> ShareState {
        let row = |id, name: &str, value: &str, rate: &str, contribution: &str, kind: &str| {
            RowData {
                id,
                name: name.into(),
                value: value.into(),
                rate: rate.into(),
                contribution: contribution.into(),
                account_kind: kind.into(),
                cost_basis: String::new(),
            }
        };
        // The example names accounts through the tax system's catalogue rather
        // than hard-coding ids, so it keeps working if the catalogue changes and
        // needs no edit at all if the whole system is swapped.
        let kinds = crate::convert::TAX_SYSTEM.account_kinds();
        let first = kinds.first().map_or("", |k| k.id);
        let taxable = kinds
            .iter()
            .find(|k| k.needs_cost_basis)
            .map_or(first, |k| k.id);
        ShareState {
            rows: vec![
                row(0, "Global Equity Fund", "10000", "7", "200", first),
                row(1, "Government Bond Fund", "5000", "3", "0", taxable),
            ],
            horizon_value: "10".into(),
            horizon_unit: "years".into(),
            plan: "deposits".into(),
            drawdown_value: "30".into(),
            drawdown_unit: "years".into(),
            withdrawal: String::new(),
            goal_target: String::new(),
            goal_kind: "topup".into(),
            strategy: String::new(),
            rate_cap: "20".into(),
            region: String::new(),
            other_income: String::new(),
            age: String::new(),
            uprate: String::new(),
        }
    }
}

/// Version marker embedded in every link. Bumped to 2 for the deposits/drawdown
/// overhaul: a v1 link's per-row `mode`/`flow` have no meaning under the new model
/// (an old `"total"` rate or `"withdraw"` flow would be misread as an annual rate
/// or a deposit), so v1 links are rejected outright and fall back to the example
/// rather than being silently mis-decoded.
const VERSION: u32 = 2;

/// The name of the fragment parameter the payload rides in (`#v=…`).
const PARAM: &str = "v";

/// The on-the-wire JSON envelope: the version tag plus the flattened state, so
/// the JSON reads `{"v":2,"rows":[…],"horizon_value":…}`.
#[derive(Serialize)]
struct WireRef<'a> {
    v: u32,
    #[serde(flatten)]
    state: &'a ShareState,
}

#[derive(Deserialize)]
struct WireOwned {
    v: u32,
    #[serde(flatten)]
    state: ShareState,
}

/// Pack the state into a fragment payload `v=<base64url>` (no leading `#`).
pub fn encode(state: &ShareState) -> String {
    let json = serde_json::to_string(&WireRef { v: VERSION, state })
        .expect("ShareState is always serializable");
    format!("{PARAM}={}", URL_SAFE_NO_PAD.encode(json))
}

/// Parse a fragment produced by [`encode`], or `None` for anything that isn't a
/// well-formed current-version link with at least one row. Accepts the string
/// with or without a leading `#`, and finds the `v` parameter among any others.
/// Never panics; a base64, JSON, or version mismatch yields `None`.
pub fn decode(fragment: &str) -> Option<ShareState> {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);

    let payload = fragment
        .split('&')
        .find_map(|pair| pair.strip_prefix("v="))?;

    let json = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let wire: WireOwned = serde_json::from_slice(&json).ok()?;

    // A different format version is rejected here rather than half-read as this one.
    if wire.v != VERSION {
        return None;
    }

    let mut state = wire.state;

    // Ids are positional bookkeeping, dropped on the way out; reassign them by
    // surviving row order so the reactive layer gets a clean, gap-free sequence.
    for (i, row) in state.rows.iter_mut().enumerate() {
        row.id = i;
    }

    if state.rows.is_empty() {
        return None;
    }
    Some(state)
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

    fn sample() -> ShareState {
        ShareState {
            rows: vec![
                row(0, "Global Equity Fund", "10000", "7", "200"),
                row(1, "Government Bond Fund", "5000", "3", "0"),
            ],
            horizon_value: "10".into(),
            horizon_unit: "years".into(),
            plan: "drawdown".into(),
            drawdown_value: "30".into(),
            drawdown_unit: "years".into(),
            withdrawal: "2000".into(),
            goal_target: "500000".into(),
            goal_kind: "topup".into(),
            strategy: "cheapest".into(),
            rate_cap: "20".into(),
            region: "narnia".into(),
            other_income: "12000".into(),
            age: "60".into(),
            uprate: "2".into(),
        }
    }

    #[test]
    fn round_trips_a_full_state() {
        let s = sample();
        let back = decode(&encode(&s)).expect("valid link decodes");
        assert_eq!(back, s);
    }

    #[test]
    fn encodes_as_a_single_url_safe_v_param() {
        let link = encode(&sample());
        assert!(link.starts_with("v="), "the payload rides in the v param");
        let payload = &link["v=".len()..];
        assert!(
            payload.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "payload {payload:?} is not clean base64url"
        );
    }

    #[test]
    fn round_trips_names_with_delimiters_and_non_ascii() {
        let mut s = sample();
        s.rows[0].name = "A & B = C ~ D # 50% \u{00a3}\u{00e9}\u{1f4c8}".into();
        let back = decode(&encode(&s)).expect("delimiters survive");
        assert_eq!(back.rows[0].name, s.rows[0].name);
    }

    #[test]
    fn a_decoded_link_accepts_the_leading_hash_too() {
        let encoded = encode(&sample());
        assert_eq!(decode(&format!("#{encoded}")), decode(&encoded));
        assert!(decode(&format!("#{encoded}")).is_some());
    }

    #[test]
    fn finds_the_v_param_among_others() {
        let encoded = encode(&sample());
        let s = decode(&format!("#utm=x&{encoded}&ref=y")).expect("v is found among others");
        assert_eq!(s.rows.len(), 2);
    }

    #[test]
    fn missing_optional_keys_fall_back_to_defaults() {
        // A v2 payload with only the version and a bare row — every other key
        // omitted — still decodes via the serde defaults.
        let payload = URL_SAFE_NO_PAD.encode(
            r#"{"v":2,"rows":[{"name":"A","value":"1000","rate":"7","contribution":"0"}]}"#,
        );
        let s = decode(&format!("v={payload}")).expect("a lone row is valid");
        assert_eq!(s.horizon_value, "10");
        assert_eq!(s.horizon_unit, "years");
        assert_eq!(s.plan, "deposits");
        assert_eq!(s.drawdown_value, "30");
        assert_eq!(s.drawdown_unit, "years");
        assert_eq!(s.withdrawal, "");
        assert_eq!(s.goal_target, "");
        assert_eq!(s.goal_kind, "topup");
        assert_eq!(s.rows.len(), 1);
        assert_eq!(s.rows[0].name, "A");
    }

    #[test]
    fn garbage_and_wrong_version_decode_to_none() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("#"), None);
        assert_eq!(decode("not-a-link"), None);
        assert_eq!(decode("foo=bar"), None);
        let junk = URL_SAFE_NO_PAD.encode("not json");
        assert_eq!(decode(&format!("v={junk}")), None);
        // A pre-overhaul v1 link (carrying the old mode/flow keys) is rejected
        // rather than mis-decoded — its rates and directions no longer mean what
        // they did.
        let v1 = URL_SAFE_NO_PAD.encode(
            r#"{"v":1,"rows":[{"name":"A","value":"1000","mode":"total","rate":"80","contribution":"0","flow":"withdraw"}]}"#,
        );
        assert_eq!(decode(&format!("v={v1}")), None);
        // A future version is likewise rejected cleanly.
        let v3 = URL_SAFE_NO_PAD.encode(
            r#"{"v":3,"rows":[{"name":"A","value":"1000","rate":"7","contribution":"0"}]}"#,
        );
        assert_eq!(decode(&format!("v={v3}")), None);
    }

    #[test]
    fn an_empty_row_list_decodes_to_none() {
        let payload = URL_SAFE_NO_PAD.encode(r#"{"v":2,"rows":[]}"#);
        assert_eq!(decode(&format!("v={payload}")), None);
    }

    #[test]
    fn ids_are_reassigned_by_position_on_decode() {
        let mut s = sample();
        s.rows[0].id = 99;
        s.rows[1].id = 42;
        let back = decode(&encode(&s)).expect("valid link");
        assert_eq!(back.rows[0].id, 0);
        assert_eq!(back.rows[1].id, 1);
    }

    #[test]
    fn the_example_is_a_valid_shareable_state() {
        let ex = ShareState::example();
        assert_eq!(ex.rows.len(), 2);
        assert_eq!(ex.rows[0].name, "Global Equity Fund");
        assert_eq!(ex.plan, "deposits");
        assert!(ex.goal_target.is_empty(), "the example leaves the goal inert");
        assert_eq!(decode(&encode(&ex)).as_ref(), Some(&ex), "the example round-trips");
    }
}

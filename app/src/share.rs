//! The shareable-link codec that makes a projection linkable.
//!
//! The whole form state — every row, the horizon, and the goal — is serialized
//! to JSON, base64url-encoded, and carried in the URL *fragment* as `#v=…` so a
//! projection can be saved, revisited, or shared without a backend or any
//! storage. A fragment is never sent in the HTTP request, so this keeps the
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
    #[serde(default)]
    pub goal_target: String,
    #[serde(default = "default_goal_kind")]
    pub goal_kind: String,
    /// What the goal is about: the sentinel `"portfolio"`, or a holding index
    /// into the filtered rows. Row order is preserved across a share/restore, so
    /// an index round-trips. Held as the raw picker string so the codec stays
    /// oblivious to `calc::Scope`.
    #[serde(default = "default_goal_scope")]
    pub goal_scope: String,
}

fn default_horizon_value() -> String {
    "10".into()
}
fn default_horizon_unit() -> String {
    "years".into()
}
fn default_goal_kind() -> String {
    "topup".into()
}
fn default_goal_scope() -> String {
    "portfolio".into()
}

impl ShareState {
    /// The built-in illustrative projection shown on a bare load — i.e. when the
    /// fragment holds no shared link (or a mangled one that [`decode`] rejects).
    /// A whole `ShareState` so the caller seeds every signal from one
    /// source instead of branching between a decoded link and inline defaults.
    /// The `RowData` ids are placeholders; the caller reassigns them as it builds
    /// the reactive rows.
    pub fn example() -> ShareState {
        let row = |id, name: &str, value: &str, mode: &str, rate: &str, contribution: &str| RowData {
            id,
            name: name.into(),
            value: value.into(),
            mode: mode.into(),
            rate: rate.into(),
            contribution: contribution.into(),
        };
        ShareState {
            rows: vec![
                row(0, "Global Equity Fund", "10000", "annual", "7", "200"),
                row(1, "Government Bond Fund", "5000", "total", "80", "0"),
            ],
            horizon_value: "10".into(),
            horizon_unit: "years".into(),
            goal_target: String::new(),
            goal_kind: "topup".into(),
            goal_scope: "portfolio".into(),
        }
    }
}

/// Version marker embedded in every link, so a future format can be told apart
/// from this one and an unrecognised version rejected cleanly rather than
/// misread.
const VERSION: u32 = 1;

/// The name of the fragment parameter the payload rides in (`#v=…`).
const PARAM: &str = "v";

/// The on-the-wire JSON envelope: the version tag plus the flattened state, so
/// the JSON reads `{"v":1,"rows":[…],"horizon_value":…}`. A borrowing form for
/// encoding and an owning one for decoding keep `encode` clone-free while letting
/// `decode` take ownership of what it parsed.
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

/// Pack the state into a fragment payload `v=<base64url>` (no leading `#`). The
/// state is serialized to JSON, then base64url-encoded (no padding) so the whole
/// payload is a single URL-safe token — no reserved character can leak out and
/// corrupt the fragment.
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

    // Pull the `v` parameter out of the `&`-separated pairs; ignore anything else.
    let payload = fragment
        .split('&')
        .find_map(|pair| pair.strip_prefix("v="))?;

    let json = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let wire: WireOwned = serde_json::from_slice(&json).ok()?;

    // A future format is rejected here rather than half-read as this one.
    if wire.v != VERSION {
        return None;
    }

    let mut state = wire.state;

    // Ids are positional bookkeeping, dropped on the way out; reassign them by
    // surviving row order so the reactive layer gets a clean, gap-free sequence.
    for (i, row) in state.rows.iter_mut().enumerate() {
        row.id = i;
    }

    // No rows means nothing to project — treat as absent so the caller loads the
    // default example instead of an empty form.
    if state.rows.is_empty() {
        return None;
    }
    Some(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: usize, name: &str, value: &str, mode: &str, rate: &str, contribution: &str) -> RowData {
        RowData {
            id,
            name: name.into(),
            value: value.into(),
            mode: mode.into(),
            rate: rate.into(),
            contribution: contribution.into(),
        }
    }

    fn sample() -> ShareState {
        ShareState {
            rows: vec![
                row(0, "Global Equity Fund", "10000", "annual", "7", "200"),
                row(1, "Government Bond Fund", "5000", "total", "80", "0"),
            ],
            horizon_value: "10".into(),
            horizon_unit: "years".into(),
            goal_target: "500000".into(),
            goal_kind: "topup".into(),
            // A holding index (not the portfolio default) so the round-trip
            // actually exercises goal_scope.
            goal_scope: "1".into(),
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
        // base64url alphabet only: no `+`, `/`, `=` padding, or other reserved
        // characters that would need further escaping in a query string.
        let payload = &link["v=".len()..];
        assert!(
            payload
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "payload {payload:?} is not clean base64url"
        );
    }

    #[test]
    fn round_trips_names_with_delimiters_and_non_ascii() {
        let mut s = sample();
        // Every reserved delimiter, plus a currency symbol and an emoji — JSON
        // string escaping and base64 both carry these through untouched.
        s.rows[0].name = "A & B = C ~ D # 50% \u{00a3}\u{00e9}\u{1f4c8}".into();
        let back = decode(&encode(&s)).expect("delimiters survive");
        assert_eq!(back.rows[0].name, s.rows[0].name);
    }

    #[test]
    fn a_decoded_link_accepts_the_leading_hash_too() {
        let encoded = encode(&sample());
        // The browser hands back the fragment with its `#`; decode must accept
        // both forms and read them identically.
        assert_eq!(decode(&format!("#{encoded}")), decode(&encoded));
        assert!(decode(&format!("#{encoded}")).is_some());
    }

    #[test]
    fn finds_the_v_param_among_others() {
        // A hand-built link with extra params still resolves `v`.
        let encoded = encode(&sample());
        let s = decode(&format!("#utm=x&{encoded}&ref=y")).expect("v is found among others");
        assert_eq!(s.rows.len(), 2);
    }

    #[test]
    fn missing_optional_keys_fall_back_to_defaults() {
        // JSON carrying only the version and a single row — every horizon/goal
        // key omitted — still decodes via the serde defaults.
        let payload = URL_SAFE_NO_PAD.encode(
            r#"{"v":1,"rows":[{"name":"A","value":"1000","mode":"annual","rate":"7","contribution":"0"}]}"#,
        );
        let s = decode(&format!("v={payload}")).expect("a lone row is valid");
        assert_eq!(s.horizon_value, "10");
        assert_eq!(s.horizon_unit, "years");
        assert_eq!(s.goal_target, "");
        assert_eq!(s.goal_kind, "topup");
        assert_eq!(s.goal_scope, "portfolio");
        assert_eq!(s.rows.len(), 1);
        assert_eq!(s.rows[0].name, "A");
    }

    #[test]
    fn garbage_and_wrong_version_decode_to_none() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("#"), None);
        assert_eq!(decode("not-a-link"), None);
        // No `v` parameter at all.
        assert_eq!(decode("foo=bar"), None);
        // Well-formed base64 of non-JSON.
        let junk = URL_SAFE_NO_PAD.encode("not json");
        assert_eq!(decode(&format!("v={junk}")), None);
        // A future version is rejected cleanly, not half-read as v1.
        let v2 = URL_SAFE_NO_PAD.encode(
            r#"{"v":2,"rows":[{"name":"A","value":"1000","mode":"annual","rate":"7","contribution":"0"}]}"#,
        );
        assert_eq!(decode(&format!("v={v2}")), None);
    }

    #[test]
    fn an_empty_row_list_decodes_to_none() {
        // Valid JSON, current version, but nothing to project.
        let payload = URL_SAFE_NO_PAD.encode(r#"{"v":1,"rows":[]}"#);
        assert_eq!(decode(&format!("v={payload}")), None);
    }

    #[test]
    fn ids_are_reassigned_by_position_on_decode() {
        let mut s = sample();
        // Ids that don't match position; the codec drops and rebuilds them.
        s.rows[0].id = 99;
        s.rows[1].id = 42;
        let back = decode(&encode(&s)).expect("valid link");
        assert_eq!(back.rows[0].id, 0);
        assert_eq!(back.rows[1].id, 1);
    }

    #[test]
    fn the_example_is_a_valid_shareable_state() {
        // The default projection must itself be a well-formed link (two rows,
        // inert goal), so seeding from it and sharing it are the same path.
        let ex = ShareState::example();
        assert_eq!(ex.rows.len(), 2);
        assert_eq!(ex.rows[0].name, "Global Equity Fund");
        assert!(ex.goal_target.is_empty(), "the example leaves the goal inert");
        assert_eq!(decode(&encode(&ex)).as_ref(), Some(&ex), "the example round-trips");
    }
}

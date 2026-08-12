# Investment Projection

A single-page tool that projects the future value of a group of investments.
Each holding is entered as its **value today** (principal plus any historical
compounding already baked in) plus a forward-looking return figure — either an
**annualised** rate or a **total return** expected over the whole horizon — and
an optional **monthly top-up** (an ongoing recurring investment). Enter a
horizon (in months or years) and it extrapolates each holding forward, sums
them into a portfolio, and charts the result.

Amounts can be typed however you like — `10000`, `10,000` and `£10,000` all
parse. The chart is scrubbable: point at it, or focus it and use the arrow
keys, to read the projected value at any month along the way. It follows your
system light/dark preference.

**Live demo:** <https://your-username.github.io/investment-projection/> (replace
`your-username` after the first deploy).

> **Not financial advice.** This is a mathematical extrapolation from a rate you
> supply — for entertainment and curiosity, not planning. Past performance does
> not predict future results.

## Architecture: Rust all the way down

Money should not be calculated in binary floating point (where `0.1 + 0.2`
famously isn't `0.3`). This app is written **entirely in Rust**, compiled to
**WebAssembly**, as a Cargo workspace of two crates:

- [`calc/`](calc/src/lib.rs) — a **pure library** that does every financial
  calculation in exact base-10 decimals (`rust_decimal`). No UI, no WASM
  specifics, no floating point. It also owns input parsing and validation, so
  invalid input comes back as a typed error naming the field that caused it.
  Unit-tested natively (`cargo test`).
- [`app/`](app/src/main.rs) — a [Leptos](https://leptos.dev) (client-side) UI
  that owns the reactive form and *formats* the `Decimal`s `calc` returns. It
  calls `calc::calculate()` directly with shared types and performs no
  arithmetic of its own.

The workspace root is a *virtual* manifest (`[workspace]` only, no package), so
both crates are named peers.

Because both layers are Rust, there is no JS↔WASM string boundary and no
JavaScript to keep in sync — the UI and the core share the same types. Nothing
is sent anywhere and nothing is stored; reload to start fresh. The page loads
with a worked example.

## Run it (Docker — no toolchain needed)

```bash
docker compose up --build
```

Then open <http://localhost:8080>. The build uses [Trunk](https://trunk-rs.github.io/trunk/)
to compile the app to WebAssembly and serves the static output via nginx. Stop
with `Ctrl+C`.

## Deploying (GitHub Pages)

Every push to `main` publishes the site to GitHub Pages via
[`.github/workflows/deploy.yml`](.github/workflows/deploy.yml). The workflow runs the
test suite (`cargo test --workspace`) and, only if it passes, builds the app with Trunk
and deploys the static output with the official Pages actions — no branch to maintain.

The one Pages-specific detail is the build flag `trunk build --release --public-url ./`.
Trunk's default output references assets with root-absolute paths (`/app-<hash>.wasm`),
which 404 when the site is served from a project subpath like
`https://<user>.github.io/investment-projection/`; `--public-url ./` makes those
references relative so they resolve under the subpath. There is no router, so relative
paths are unambiguous.

First-time setup: create a public repo, push, then set **Settings → Pages → Source** to
**GitHub Actions** and re-run the workflow.

## How the numbers work

The **horizon** (years or months) is converted to whole months in `Decimal`
(fractional periods are rounded there, not in the UI), clamped to a minimum of
1 month and a maximum of 1200 (100 years). Each holding then needs a single
**annualised rate**, derived from whichever return figure was entered:

- **Annualised input** — the rate you enter *is* the annualised rate, used
  directly.
- **Total-return input** — the figure is the cumulative return expected over the
  whole projection horizon, so the equivalent annualised rate is derived as
  `(1 + total)^(12 / horizonMonths) − 1`. This makes the holding land exactly on
  `value × (1 + total)` at the horizon.

The projection compounds each holding's value-today forward at that annualised
rate: the monthly factor is `(1 + annual)^(1/12)`, and each month the running
value is compounded and then the **monthly top-up** is added at month end (so
today's value is unaffected — the money isn't invested yet). Month 0 is today's
value, so the series has `horizonMonths + 1` points; the portfolio line is the
sum across holdings. Fractional exponents use `rust_decimal`'s decimal `powd`,
so no `f64` enters the calculation.

**Projected growth** is reported as *returns only*: the final value less both
today's value and the total you contribute along the way, so your own deposits
are never counted as gains. Its percentage is that gain over the capital
actually deployed (today's value plus all contributions) — a simple return on
capital, not a money-weighted IRR. The UI states that denominator ("of £39,000
put in") rather than leaving a bare percentage to be interpreted.

The per-holding table shows top-ups as their own column for the same reason.
A row reading `£10,000 → £53,881.86 at +7.00%` looks wrong on its own —
£10,000 at 7% for ten years is £19,671 — until you can see the £24,000 of
monthly top-ups that bridge the gap.

## Accessibility

Accessibility here is deliberate rather than incidental. The following are
implemented and have been checked against the running app — this is not a claim
of full WCAG conformance, which would need a proper audit:

- Every colour pair passes AA contrast in **both** light and dark themes.
- Validation errors name the control they belong to (`aria-invalid` +
  `aria-describedby`), rather than stranding a sentence at the foot of the form.
  Announcements are debounced and polite, so a screen reader isn't interrupted
  mid-word while you are still typing.
- A half-typed number never blanks the results — the last good figures stay on
  screen, dimmed and marked `aria-busy`.
- Removing a holding moves focus to the next remove button (or to
  "+ Add investment"), so keyboard users aren't dropped back at the top of the
  page.
- The chart is exposed as a slider with `aria-valuetext`, so its intermediate
  values are reachable by keyboard, not just by hovering a mouse.
- Gain and loss are distinguished by an explicit `+`/`-` sign and by the label
  wording, not by green-vs-red alone.

## Developing

A [devcontainer](.devcontainer/devcontainer.json) (Rust + `wasm32` target +
Trunk) is provided. Inside it, or on any machine with Rust, the `wasm32` target,
and Trunk installed:

```bash
./dev.sh          # trunk serve with live reload on http://localhost:8080
cargo test -p calc   # run the calculation-core unit tests (native, fast)
cargo test           # the above plus the format/chart tests in app/
```

## Layout

| Path                 | What it is                                                        |
| -------------------- | ---------------------------------------------------------------- |
| `Cargo.toml`         | Virtual workspace manifest (`[workspace]` only) + shared profile |
| `calc/src/lib.rs`    | Pure exact-decimal calculation core + its unit tests             |
| `app/src/main.rs`    | Leptos UI: mount + the reactive form/results components          |
| `app/src/format.rs`  | `Decimal` → display-string formatting (with tests)              |
| `app/src/chart.rs`   | Hand-built SVG portfolio chart + its plot geometry (with tests)  |
| `app/index.html`     | Trunk entry point (Trunk runs from `app/`)                       |
| `app/styles.css`     | Styles; single source of the light and dark palettes, referenced by the chart |
| `Dockerfile`         | Multi-stage: Trunk build → serve with nginx                      |
| `docker-compose.yml` | One-command run                                                  |
| `dev.sh`             | Local `trunk serve` helper (`cd`s into `app/`)                   |
| `.github/workflows/deploy.yml` | GitHub Actions: test, then build with Trunk and deploy to Pages |

//! Binary entry point: mount the [`App`] component to the document body. All the
//! interesting code lives in the `app` library crate (`app/src/lib.rs`) so the
//! headless-browser tests in `app/tests/ui.rs` can mount `<App/>` directly.

use app::App;
use leptos::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App/> });
}

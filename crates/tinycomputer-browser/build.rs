//! Selects opt-in browser tests at compile time.

fn main() {
    println!("cargo:rustc-check-cfg=cfg(live_browser_tests)");
    println!("cargo:rerun-if-env-changed=TINYCOMPUTER_LIVE_BROWSER_TESTS");

    if std::env::var_os("TINYCOMPUTER_LIVE_BROWSER_TESTS").is_some() {
        println!("cargo:rustc-cfg=live_browser_tests");
    }
}

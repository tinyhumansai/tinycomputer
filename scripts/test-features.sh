#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 scripts/test-e2e-version.py
python3 scripts/test-source-line-endings.py
case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*)
    # The module tests each create multi-thread Tokio runtimes, and trace-sink
    # coverage also owns worker threads. Serial test execution avoids a
    # Windows test-process access violation during concurrent runtime teardown.
    export RUST_TEST_THREADS=1
    ;;
esac
cargo test --locked --workspace
cargo test --locked --workspace --all-features
RUST_TEST_THREADS=1 TINYCOMPUTER_LIVE_BROWSER_TESTS=1 cargo test --locked -p tinycomputer-browser --features agent-browser live_
RUST_TEST_THREADS=1 TINYCOMPUTER_LIVE_BROWSER_EGRESS_TESTS=1 cargo test --locked -p tinycomputer --features static-link tinybus_module::browser_egress_live_tests -- --nocapture
cargo test --locked -p tinycomputer --features static-link
cargo run --locked -p tinycomputer-examples --bin basic
forbidden="$(cargo tree -p tinycomputer-bus -e normal,build --prefix none | grep -Ei 'tinybus|tokio|reqwest|ureq|hyper|rusqlite|git2|agent-desktop' || true)"
[[ -z "$forbidden" ]] || { echo "$forbidden" >&2; exit 1; }
case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*) ;;
  *)
    python3 scripts/test-package-layout.py
    python3 scripts/test-published-release.py
    ;;
esac

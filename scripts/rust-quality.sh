#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo fmt --manifest-path tests/fixtures/rendering-consumer/Cargo.toml -- --check
cargo clippy -p ved-core --all-targets --no-default-features --locked -- -D warnings
cargo test -p ved-core --no-default-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc -p ved-core --no-default-features --no-deps --locked
python3 scripts/check-core-features.py

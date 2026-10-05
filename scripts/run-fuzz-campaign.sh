#!/usr/bin/env bash
# Bounded fuzz campaign runner with artifact retention.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ARTIFACT_DIR="${VED_FUZZ_ARTIFACT_DIR:-$ROOT_DIR/target/fuzz-artifacts}"
mkdir -p "$ARTIFACT_DIR"

MAX_TIME="${FUZZ_MAX_TOTAL_TIME:-30}"
TIMEOUT="${FUZZ_TIMEOUT:-5}"
MAX_LEN="${FUZZ_MAX_LEN:-1048576}"
RSS_LIMIT_MB="${FUZZ_RSS_LIMIT_MB:-2048}"

TARGETS=("parsers" "paths" "manifest_roundtrip")

echo "Running fuzz campaign (max_time=${MAX_TIME}s per target)..."

if command -v cargo-fuzz >/dev/null 2>&1 && rustup run nightly cargo fuzz --version >/dev/null 2>&1; then
    echo "Using instrumented cargo-fuzz with nightly toolchain"
    for target in "${TARGETS[@]}"; do
        echo "Campaign: $target"
        TEMP_CORPUS="$(mktemp -d)"
        if [ -d "fuzz/seeds/$target" ]; then
            cp -r "fuzz/seeds/$target/"* "$TEMP_CORPUS/" 2>/dev/null || true
        fi
        
        set +e
        cargo +nightly fuzz run "$target" "$TEMP_CORPUS" -- \
            -max_total_time="$MAX_TIME" \
            -timeout="$TIMEOUT" \
            -max_len="$MAX_LEN" \
            -rss_limit_mb="$RSS_LIMIT_MB" \
            -artifact_prefix="$ARTIFACT_DIR/" \
            2>&1 | tee "$ARTIFACT_DIR/${target}.log"
        STATUS="${PIPESTATUS[0]}"
        set -e
        
        rm -rf "$TEMP_CORPUS"
        if [ "$STATUS" -ne 0 ]; then
            echo "Fuzz target $target reported failure (status $STATUS)"
            exit "$STATUS"
        fi
    done
else
    echo "cargo-fuzz or nightly not detected; running deterministic bounded smoke tests"
    for target in "${TARGETS[@]}"; do
        echo "Smoke test: $target"
        cargo run --locked --manifest-path fuzz/Cargo.toml --bin "$target" -- \
            "fuzz/seeds/$target" \
            -runs=100 \
            -max_len=512 \
            -timeout="$TIMEOUT" \
            2>&1 | tee "$ARTIFACT_DIR/${target}-smoke.log"
    done
fi

echo "Fuzz campaign completed successfully. Artifacts retained in $ARTIFACT_DIR"

#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# scripts/release.sh — Bump version, verify, tag, push, and publish
# ─────────────────────────────────────────────────────────────────────────────

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [ -d "$HOME/.cargo/bin" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

if [ -f ".ved/.ved-version" ]; then
    CURRENT_VERSION="$(tr -d '[:space:]' < .ved/.ved-version)"
elif [ -f ".ved-version" ]; then
    CURRENT_VERSION="$(tr -d '[:space:]' < .ved-version)"
else
    CURRENT_VERSION="0.1.0"
fi

# 1. Parse target version
BUMP_TARGET="${1:-patch}"
NEW_VERSION=""

IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"

case "$BUMP_TARGET" in
    patch)
        NEW_VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))"
        ;;
    minor)
        NEW_VERSION="${MAJOR}.$((MINOR + 1)).0"
        ;;
    major)
        NEW_VERSION="$((MAJOR + 1)).0.0"
        ;;
    v*.*.*|*.*.*)
        NEW_VERSION="${BUMP_TARGET#v}"
        ;;
    *)
        echo "Usage: $0 [patch | minor | major | X.Y.Z]" >&2
        exit 1
        ;;
esac

echo "==> Preparing release: v${CURRENT_VERSION} -> v${NEW_VERSION}"

if [ "${TARGET:-}" = "patch" ] || [ "${TARGET:-}" = "minor" ] || [ "${TARGET:-}" = "major" ]; then
    unset TARGET
fi

# 2. Pre-flight checks
if [ -n "$(git status --porcelain)" ]; then
    echo "Error: Working directory has uncommitted changes. Stash or commit first." >&2
    git status -s >&2
    exit 1
fi

BRANCH="$(git rev-parse --abbrev-ref HEAD)"

# 3. Run test verification
echo "==> Running test suite..."
unset TARGET DEPLOYMENT_TARGET
bash scripts/rust-quality.sh
python3 scripts/check-advisory-exceptions.py
cargo deny --locked check
cargo deny --manifest-path fuzz/Cargo.toml --locked --config deny.toml check
bash scripts/package-check.sh
if [ -f "./.venv/bin/pytest" ]; then
    ./.venv/bin/pytest tests/ -q
fi

# 4. Bump version in tracked files
echo "==> Updating version to ${NEW_VERSION}..."
mkdir -p .ved
echo "${NEW_VERSION}" > .ved/.ved-version
rm -f .ved-version .velos-version

python3 -c "
import re
with open('crates/ved/Cargo.toml', 'r') as f:
    content = f.read()
content = re.sub(r'version = \"[0-9.]+\"', f'version = \"${NEW_VERSION}\"', content, count=1)
with open('crates/ved/Cargo.toml', 'w') as f:
    f.write(content)
"

python3 -c "
import re
with open('vew', 'r') as f:
    content = f.read()
content = re.sub(r'VERSION=\"[0-9.]+\"', f'VERSION=\"${NEW_VERSION}\"', content, count=1)
with open('vew', 'w') as f:
    f.write(content)
"
cp vew crates/ved/assets/vew

# Refresh Cargo.lock and build release binary
echo "==> Building release binary..."
cargo build --release
python3 scripts/check-advisory-exceptions.py
cargo deny --locked check
cargo deny --manifest-path fuzz/Cargo.toml --locked --config deny.toml check
bash scripts/rust-quality.sh
bash scripts/package-check.sh

# 5. Commit and tag
echo "==> Committing and tagging v${NEW_VERSION}..."
git add .ved/.ved-version crates/ved/Cargo.toml vew crates/ved/assets/vew Cargo.lock
git commit -m "chore(release): bump version to ${NEW_VERSION}"
git tag -a "v${NEW_VERSION}" -m "Release v${NEW_VERSION}"

# 6. Push to remote
echo "==> Pushing to origin..."
git push origin "$BRANCH"
git push origin "v${NEW_VERSION}"

# 7. Update local installation if Homebrew path exists
BREW_BIN="/opt/homebrew/bin/ved"
if [ -L "$BREW_BIN" ]; then
    CELLAR_TARGET="$(readlink -f "$BREW_BIN" 2>/dev/null || readlink "$BREW_BIN" 2>/dev/null || true)"
    if [ -n "$CELLAR_TARGET" ] && [ -f "$CELLAR_TARGET" ]; then
        echo "==> Updating local Homebrew binary at ${CELLAR_TARGET}..."
        chmod +w "$CELLAR_TARGET" 2>/dev/null || true
        cp "target/release/ved" "$CELLAR_TARGET" 2>/dev/null || true
        chmod -w "$CELLAR_TARGET" 2>/dev/null || true
    fi
fi

echo ""
echo "============================================================"
echo " Successfully published v${NEW_VERSION}!"
echo " GitHub Actions release workflow is running."
echo " Watch progress with: gh run watch"
echo "============================================================"

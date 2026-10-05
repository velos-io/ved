#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [ -d "$HOME/.cargo/bin" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

if cargo +stable --version >/dev/null 2>&1; then
  CARGO_CMD=(cargo +stable)
elif command -v rustup >/dev/null 2>&1 && rustup run stable cargo --version >/dev/null 2>&1; then
  CARGO_CMD=(rustup run stable cargo)
elif [ -x "$HOME/.cargo/bin/rustup" ] && "$HOME/.cargo/bin/rustup" run stable cargo --version >/dev/null 2>&1; then
  CARGO_CMD=("$HOME/.cargo/bin/rustup" run stable cargo)
else
  CARGO_CMD=(cargo)
fi

# Cargo 1.90+ archives all siblings. Verify those exact archives rather than
# accidentally selecting older crates.io content with the same package version.
package_target=$("${CARGO_CMD[@]}" metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
"${CARGO_CMD[@]}" package --workspace --locked --allow-dirty --no-verify
package_fixture=$(mktemp -d)
trap 'rm -rf "$package_fixture"' EXIT
python3 - "$PWD" "$package_fixture" "$package_target" <<'PY'
import pathlib, sys, tarfile, tomllib
root, fixture, target = map(pathlib.Path, sys.argv[1:])
packages = {}
for crate in ('ved-process', 'ved-core', 'ved'):
    source = tomllib.loads((root / 'crates' / crate / 'Cargo.toml').read_text())
    version = source['package']['version']
    archive = target / 'package' / f'{crate}-{version}.crate'
    with tarfile.open(archive) as contents:
        contents.extractall(fixture, filter='data')
    extracted = fixture / f'{crate}-{version}'
    manifest = tomllib.loads((extracted / 'Cargo.toml').read_text())
    assert manifest['package']['name'] == crate
    assert manifest['package']['version'] == version
    (extracted / 'original-lock.toml').write_bytes((extracted / 'Cargo.lock').read_bytes())
    packages[crate] = (version, extracted, manifest)
for crate, (_, _, manifest) in packages.items():
    for dependency, data in manifest.get('dependencies', {}).items():
        if dependency in packages:
            assert data['version'] == packages[dependency][0], (crate, dependency, data)
ved = packages['ved'][1]
assert (ved / 'assets' / 'vew').read_bytes() == (root / 'crates/ved/assets/vew').read_bytes()
assert (ved / 'build_support.rs').is_file()
lines = ['[patch.crates-io]']
for crate, (_, extracted, _) in packages.items():
    # JSON string escaping is also valid TOML basic-string escaping for paths.
    import json
    lines.append(f'{crate} = {{ path = {json.dumps(str(extracted))} }}')
(fixture / 'archive-overrides.toml').write_text('\n'.join(lines) + '\n')
(fixture / 'cli-path').write_text(str(ved))
print('Validated archive names, versions, sibling requirements and embedded build inputs.')
PY
package_overrides="$package_fixture/archive-overrides.toml"
package_cli=$(cat "$package_fixture/cli-path")
export CARGO_TARGET_DIR="$package_fixture/target"
# Resolving explicit local overrides replaces registry checksums in temporary
# extracted lockfiles only. Subsequent builds and installation remain locked.
for package_manifest in "$package_fixture"/ved-[0-9]*/Cargo.toml "$package_fixture"/ved-core-*/Cargo.toml "$package_fixture"/ved-process-*/Cargo.toml; do
  case "$package_manifest" in
    */ved-core-*) "${CARGO_CMD[@]}" update --offline -p ved-process --manifest-path "$package_manifest" --config "$package_overrides" ;;
    */ved-process-*) ;;
    *) "${CARGO_CMD[@]}" update --offline -p ved-core -p ved-process --manifest-path "$package_manifest" --config "$package_overrides" ;;
  esac
  python3 - "$package_manifest" <<'PYLOCK'
import pathlib, sys, tomllib
root = pathlib.Path(sys.argv[1]).parent
def external_graph(path):
    packages = tomllib.loads(path.read_text())['package']
    return {(p['name'], p['version'], p.get('source'), p.get('checksum'))
            for p in packages if p['name'] not in {'ved', 'ved-core', 'ved-process'}}
assert external_graph(root / 'Cargo.lock') == external_graph(root / 'original-lock.toml'), 'Archive non-sibling locked dependency graph changed'
PYLOCK
  case "$package_manifest" in
    */ved-process-*) "${CARGO_CMD[@]}" build --locked --manifest-path "$package_manifest" ;;
    *) "${CARGO_CMD[@]}" build --locked --manifest-path "$package_manifest" --config "$package_overrides" ;;
  esac
done
"${CARGO_CMD[@]}" install --locked --debug --path "$package_cli" --root "$package_fixture/install" --config "$package_overrides"
package_report=${VED_PACKAGE_REPORT_DIR:-"$PWD/target/package-validation"}
python3 scripts/release-validation.py --ved "$package_fixture/install/bin/ved" --smoke-only --report-dir "$package_report"
printf '%s\n' 'Exact archive build/install verification passed with local sibling overrides; registry publication readiness is a separate check.'

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
import pathlib, sys, tarfile

try:
    import tomllib
    def parse_toml(text):
        return tomllib.loads(text)
except ModuleNotFoundError:
    try:
        import tomli as tomllib
        def parse_toml(text):
            return tomllib.loads(text)
    except ModuleNotFoundError:
        import re
        def parse_toml(text):
            data = {}
            current = data
            for line in text.splitlines():
                line = line.strip()
                if not line or line.startswith('#'):
                    continue
                if line.startswith('[') and line.endswith(']'):
                    sec = line[1:-1].strip()
                    parts = sec.split('.')
                    d = data
                    for p in parts:
                        d = d.setdefault(p, {})
                    current = d
                    continue
                if '=' in line:
                    k, v = line.split('=', 1)
                    k, v = k.strip(), v.strip()
                    if v.startswith('{') and v.endswith('}'):
                        tbl = {}
                        for m in re.finditer(r'([a-zA-Z0-9_-]+)\s*=\s*(?:"([^"]*)"|([^,\s}]+))', v[1:-1]):
                            key = m.group(1)
                            val = m.group(2) if m.group(2) is not None else m.group(3)
                            tbl[key] = val
                        current[k] = tbl
                    elif v.startswith('"') and v.endswith('"'):
                        current[k] = v[1:-1]
                    elif v == 'true':
                        current[k] = True
                    elif v == 'false':
                        current[k] = False
            return data

root, fixture, target = map(pathlib.Path, sys.argv[1:])
packages = {}
for crate in ('ved-process', 'ved-core', 'ved'):
    source = parse_toml((root / 'crates' / crate / 'Cargo.toml').read_text())
    version = source['package']['version']
    archive = target / 'package' / f'{crate}-{version}.crate'
    with tarfile.open(archive) as contents:
        if hasattr(tarfile, 'data_filter'):
            contents.extractall(fixture, filter='data')
        else:
            contents.extractall(fixture)
    extracted = fixture / f'{crate}-{version}'
    manifest = parse_toml((extracted / 'Cargo.toml').read_text())
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
import pathlib, sys

root = pathlib.Path(sys.argv[1]).parent
def external_graph(path):
    packages = []
    current = None
    for line in path.read_text().splitlines():
        line = line.strip()
        if line == '[[package]]':
            if current:
                packages.append(current)
            current = {}
        elif current is not None and '=' in line:
            k, v = line.split('=', 1)
            k = k.strip()
            v = v.strip().strip('"')
            current[k] = v
    if current:
        packages.append(current)
    return {(p['name'], p['version'], p.get('source'), p.get('checksum'))
            for p in packages if p['name'] not in {'ved', 'ved-core', 'ved-process'}}
    diff1 = external_graph(root / 'Cargo.lock') - external_graph(root / 'original-lock.toml')
    diff2 = external_graph(root / 'original-lock.toml') - external_graph(root / 'Cargo.lock')
    assert not diff1 and not diff2, f'Archive non-sibling locked dependency graph changed: added={diff1}, removed={diff2}'
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

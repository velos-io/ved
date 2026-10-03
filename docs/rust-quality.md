# Rust quality and release policy

Rust 1.88.0 is the minimum supported version and default developer toolchain.
Workspace crates inherit `rust-version`; PR and push CI checks this toolchain and
also tests current stable on Linux and macOS. Toolchain/lockfile updates must pass
the whole quality suite; dependency metadata alone does not establish compatibility.
Use rustup's cargo shim (normally `~/.cargo/bin/cargo`) to honor rust-toolchain.toml.

Run `bash scripts/rust-quality.sh`: format, locked clippy on all targets with
warnings denied, locked workspace tests including doctests, and rustdoc with
warnings denied. The same gate checks the rendering-only core with default
features disabled, formats the standalone consumer, and checks its independent
locked dependency graph and exact artifact parity across both feature modes.
Tests use explicit configuration snapshots and isolated CLI child
environments with recording fake Docker/Kubernetes executables. They do not need
a daemon or cluster. External kubectl manifest validation is ignored by default;
run it deliberately with `cargo test -p ved-core --test test_k8s_serialization
complete_generated_trees_build_with_real_kustomize -- --ignored --exact`.
Each external command has a 10-second deadline and fails if kubectl is missing.
Three task-history assertions are also ignored by default because `agent-work/`
history is local and absent from a checkout. Run those local operations checks with
`cargo test -p ved --test test_task_registry -- --ignored`; they do not test the CLI.
Example regressions use explicit base configuration and committed fixtures,
independent of ignored developer overrides and previously generated artifacts.

`cargo deny --locked check` enforces the SPDX licence allowlist, known registries,
no wildcard dependencies and RustSec advisories. The permissive NCSA licence is
allowed for the libFuzzer development tooling alongside the existing allowlist. Duplicate versions warn. Yanked
crates, unsoundness and unmaintained advisories (including transitives) fail.
Four individually reviewed maintenance-only advisory IDs are excepted below.
The exceptions do not exempt the crates from new security or maintenance advisories.
Unused exceptions fail, so an upstream removal requires cleaning up this policy.
Before Cargo Deny, run `python3 scripts/check-advisory-exceptions.py` (Python 3.11+).
CI and local release automation run this guard and apply Cargo Deny to both
workspaces (`cargo deny --manifest-path fuzz/Cargo.toml --locked --config deny.toml check`
for the fuzz workspace). It checks both the workspace and
fuzz locked graphs across all targets, requiring the exact reviewed package versions,
registry sources and immediate reverse dependency edges. The exceptions expire
at **2027-01-09 00:00 UTC**; expiry or dependency drift fails until explicitly reviewed.
`python3 -m unittest discover -s tests -p test_advisory_exceptions.py` checks this
boundary, including expired and changed graphs.

### Maintenance advisory exceptions (reviewed 2026-10-18)

| Advisory | Locked dependency | Dependency route | Owner | Review date | Removal criteria |
| --- | --- | --- | --- | --- | --- |
| [RUSTSEC-2023-0089](https://rustsec.org/advisories/RUSTSEC-2023-0089) | atomic-polyfill 1.0.3 | heapless → postcard → pagable → Starlark | repository-maintainers | 2027-01-09 UTC | Upstream Starlark dependency replacement |
| [RUSTSEC-2024-0388](https://rustsec.org/advisories/RUSTSEC-2024-0388) | derivative 2.2.0 | Starlark / starlark_syntax | repository-maintainers | 2027-01-09 UTC | Upstream Starlark macro derive replacement |
| [RUSTSEC-2025-0057](https://rustsec.org/advisories/RUSTSEC-2025-0057) | fxhash 0.2.1 | starlark_map | repository-maintainers | 2027-01-09 UTC | Upstream starlark_map hash replacement |
| [RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436) | paste 1.0.15 | Starlark | repository-maintainers | 2027-01-09 UTC | Upstream Starlark dependency cleanup |
| [RUSTSEC-2024-0449](https://rustsec.org/advisories/RUSTSEC-2024-0449) | unsafe-libyaml 0.2.11 | serde_yaml | repository-maintainers | 2027-01-09 UTC | Breaking release 0.2.0 decoupled domain error |

These advisories are classified by RustSec as informational unmaintained notices,
with no patched versions. The current published Starlark 0.14.2 and `serde_yaml` 0.9.34
require these transitives. Alternatives to `serde_yaml` (`serde_yaml_ng` and `serde-saphyr`)
were evaluated: `serde_yaml_ng` continues to use `unsafe-libyaml`, reproducing the identical
unmaintained advisory without benefit; `serde-saphyr` breaks the public API contract of
`VedError::Yaml(serde_yaml::Error)`. The policy explicitly accepts this bounded maintenance
debt to preserve public API stability in 0.1.x while mitigating risks through `LoadSession`
bounded source readers. Reviewers must reassess all exceptions before 2027-01-09 UTC.
Cargo-deny prints the excepted advisories as notes.
Cargo-deny 0.20.2 scopes exceptions by advisory ID, not package version; Cargo.lock
records the reviewed versions, and updates require review and the full quality suite.

`bash scripts/package-check.sh` uses Cargo 1.90+ workspace packaging and Python
3.11+ to create archives, check package/dependency versions and required assets,
then build and install the extracted CLI with explicit local sibling overrides.
Temporary extracted lockfiles resolve those overrides before locked verification.
This verifies exact local archive content without substituting older registry
crates that share a version; registry publication readiness remains a separate
check. It creates archives locally and does not publish. Stable CI runs packaging on Linux/macOS; the separate 1.88 job tests the
MSRV. Tag releases call quality CI before building binaries. Local release checks
run before and after the version/lock refresh. The version bump and refreshed
Cargo.lock are committed together.

Builds embed the committed `crates/ved/assets/vew` wrapper directly and do not
copy or modify source assets. Keep the repository-root `vew` copy in sync explicitly;
a repository parity regression detects drift without coupling archive builds to it.
Set `SOURCE_DATE_EPOCH` to an ASCII decimal Unix timestamp between 0 and
253402300799 (inclusive) for deterministic UTC `BUILD_TIMESTAMP` metadata. Changes
to this environment variable invalidate Cargo's build-script cache. Invalid supplied
values fail the build; an unset value retains the current UTC timestamp for local
development. This controls timestamp metadata, not complete binary reproducibility.

Workspace lint policy denies unsafe code. The ConfigSnapshot Starlark derive is
the sole documented local exception: it generates a marker impl for a type whose
fields own static maps. New exceptions require a local rationale and review;
handwritten signal handling and euid queries use safe library wrappers.

Process cancellation policy belongs to the application. `ved-process::Runner`
no longer registers SIGINT/SIGTERM handlers implicitly; its existing constructor
and cancellation methods remain available. Library consumers that want signal
cancellation must register their own handlers and pass a `CancellationToken` to
each runner. `CancellationToken::from_flag` accepts an application-owned
`Arc<AtomicBool>` for integration with safe signal registration libraries.
Standalone tokens still support explicit cancellation without any signal policy.

On Unix, the CLI registers SIGINT/SIGTERM once per invocation and shares its token
with command execution and explicit or automatic builds. Cancellation terminates
active subprocess groups and prevents pending subprocesses from starting. The
registration guard unregisters its callbacks on return, including setup failures;
this does not promise restoration of the operating system's default signal action.
Timeout, output-limit and cancellation errors retain their distinct types.
Core Git catalog synchronization also provides additive token-aware host APIs;
legacy entrypoints use standalone tokens. Signal regressions run in dedicated
child processes on Unix with readiness markers and deadlines.

Release integration uses exact Compose, kubectl and embedded Kustomize versions
from `scripts/validation-tools.json`. Install the standalone tools with upstream
SHA256 verification, then validate generated fixtures offline:

```sh
python3 scripts/install-validation-tools.py --destination /tmp/ved-validation-tools --report-dir target/release-evidence/tools
cargo build --locked --bin ved
python3 scripts/release-validation.py --ved target/debug/ved --compose /tmp/ved-validation-tools/docker-compose --kubectl /tmp/ved-validation-tools/kubectl --report-dir target/release-evidence/offline
```

The harness accepts explicit executable paths, isolates configuration and
environment, and bounds each command's time and output. It runs Compose `config`
and kubectl `kustomize`; these checks prove offline parser acceptance only.
They do not prove Kubernetes schema/admission acceptance, image availability,
application health or runtime readiness. No daemon or cluster is required.
The dedicated quality job also explicitly enables the empty/multi-group real
Kustomize regression. All validation failures block dependent release builds.

`scripts/package-check.sh` installs the extracted Cargo archive CLI with local
sibling overrides and runs version/help/representative compilation through the
same isolated harness. Evidence defaults to `target/package-validation`; set
`VED_PACKAGE_REPORT_DIR` to retain it elsewhere. Release tarballs contain exactly
one executable `ved`. Validate the extracted binary with
`python3 scripts/release-validation.py --archive PATH.tar.gz --report-dir target/archive-validation`.
Cross-built archives use `--content-only`, which records archive inspection
without claiming executable smoke evidence; native archives execute the smoke.
CI uploads version reports, captured successful output and failure diagnostics
even when validation fails. A changed workflow requires hosted execution to
establish hosted evidence; local execution alone does not establish that.

To update tool pins, edit the shared JSON, download the exact releases and their
official checksums, rerun the harness and Python failure regressions, and review
the retained actual-version/hash reports. Missing tools, mismatched versions,
checksum failures and failed parser commands fail rather than skip the gate.

Run property regressions with `cargo test -p ved-core --test quality_properties
--locked`. `cargo bench -p ved-core --bench quality --locked` measures parse/emit
and the build scheduler using isolated fake build tools. Baselines are diagnostic,
not pass/fail timing thresholds. No production optimization is justified by them.
See `fuzz/README.md` and `scripts/run-fuzz-campaign.sh` for bounded fuzz smoke runs and sanitizer-enabled nightly campaigns (`.github/workflows/fuzz.yml`).

Policy/tool references: [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html),
[cargo-deny advisories](https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html),
[cargo-deny licences](https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html),
[bounded fuzz CI](https://rust-fuzz.github.io/book/cargo-fuzz/ci.html).

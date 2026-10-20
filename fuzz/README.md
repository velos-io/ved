# Bounded quality fuzzing

Install `cargo-fuzz`, then use a nightly toolchain:

```sh
cargo +nightly fuzz run parsers fuzz/seeds/parsers -- -max_total_time=30 -timeout=5
cargo +nightly fuzz run paths fuzz/seeds/paths -- -max_total_time=30 -timeout=5
cargo +nightly fuzz run manifest_roundtrip fuzz/seeds/manifest_roundtrip -- -max_total_time=30 -timeout=5
```

Targets are pure: no deployment processes, global environment mutation, or filesystem writes. Parser inputs exercise shorthand, YAML wire parsing plus semantic validation, and explicit environment interpolation. The path target verifies the validated identifier boundary used before artifact access; descriptor-relative symlink confinement remains covered by the filesystem regression suite. Roundtrip checks preserve scalar strings and semantic YAML values. Retain minimized failures as regression cases. The independent fuzz workspace keeps libFuzzer out of release packages.

Without nightly/cargo-fuzz, a finite link/runtime smoke check can run each binary with `cargo run --manifest-path fuzz/Cargo.toml --bin <target> -- fuzz/seeds/<target> -runs=100 -max_len=512`. This does **not** provide sanitizer or coverage instrumentation and is not a replacement for the bounded nightly campaigns above.

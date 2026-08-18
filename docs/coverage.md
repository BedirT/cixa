# Security-Critical Coverage

The target in `goal.md` is at least 90% branch coverage for policy and ledger code unless a specific exception is documented. The current local measurement is an explicit exception, not a claim that the target passed.

Measured on 2026-08-16 with nightly Rust, `llvm-tools-preview`, and `cargo-llvm-cov 0.8.7`:

- `crates/domain/src/lib.rs` branch coverage: **50.17%** (`294 / 586`);
- line coverage: **68.44%**;
- function coverage: **59.52%**.

Reproduce it with:

```bash
rustup toolchain install nightly --profile minimal --component llvm-tools-preview
cargo install cargo-llvm-cov --locked
./scripts/coverage
```

## Exception Rationale

The 90% branch target is not enforced for the `0.1` reference release because the core currently combines policy, ledger, persistence, provider simulation, IPC types, and owner operations in one file. Raising the aggregate number immediately would reward low-value structural tests and generated branches rather than improve payment safety. The canonical gate instead requires focused authority, ambiguity, persistence, reconciliation, strict-schema, redaction, dashboard, socket-flood, and end-to-end tests plus executable fuzz targets.

The separate `./scripts/verify-container` gate builds both Docker targets, initializes disposable volumes, checks owner-service health, creates a synthetic scoped capability, and calls MCP tools from the read-only, network-disabled agent container. It proves the packaged UID, volume, socket, and token boundary that the native unit suite cannot exercise.

This exception must be removed before describing the project as production-ready. The next coverage work should split policy and ledger modules, measure them separately, and bring each security-critical module to 90% branch coverage with behavior-driven boundary tests. Any regression below the measured 50.17% baseline should be investigated even while the exception remains.

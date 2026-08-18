# Contributing

## Security Boundary First

Read `README.md`, `THREAT_MODEL.md`, `docs/security-model.md`, and the nearest ADR before editing security-sensitive code. Do not weaken a denial into an approval to make a test green. Add a regression test for every policy, authorization, state-machine, redaction, or reconciliation change.

Never use real financial credentials. Use synthetic secrets and run the generated-artifact canary scan. Do not reverse engineer or automate a private issuer API. Do not add telemetry, hosted dependencies, public listeners, or third-party dashboard assets without an explicit ADR and threat-model update.

## Local Checks

```bash
cargo fmt --all
cargo test --workspace
npm ci
npm run build
npm test
PYTHONPATH=packages/sdk-python python3 -m unittest discover -s packages/sdk-python/tests
./scripts/verify
./scripts/verify-container
```

Keep `Cargo.lock` and `package-lock.json` committed. Prefer a small dependency-free implementation when it is safer. Do not use floating point for money. Preserve unrelated worktree changes and review the exact diff before committing.

## Changes and Reports

Explain the trust boundary, failure mode, tests, residual risks, and documentation impact in the change description. Never include raw credentials, owner tokens, agent tokens, audit keys, or browser artifacts in commits, issues, or pull requests. Public release and external transactions remain owner decisions.

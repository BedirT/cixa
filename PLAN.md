# Implementation Plan

## Checkpoint 1 - Research and Architecture

Completed. Current official KOHO, PCI SSC, MCP, Playwright, and OS credential-store sources are recorded in [docs/research.md](docs/research.md). The Rust boundary, manual-provider decision, CVV strategy, local IPC, checkout trust model, and ledger choice are recorded in `docs/adr/`.

## Checkpoint 2 - Core Domain and Simulator

Completed. `crates/domain` implements integer money, policy decisions, transaction transitions, budget usage, provider outcomes, manual adapter metadata, verified versus unverified income, simulated holds and charges, receipt redaction, capability hashes, and a keyed audit chain.

## Checkpoint 3 - Daemon and Authorization

Completed. `apps/daemon` provides persisted state, atomic state writes, private file permissions on Unix, loopback local IPC through a Unix-domain socket, separate owner and agent operations, expiration and revocation fields, emergency stop, and restart loading with audit verification.

## Checkpoint 4 - Agent Interfaces

Completed. The v1 JSON IPC is exposed through the CLI, `packages/sdk-typescript`, `packages/sdk-python`, and the MCP v2 TypeScript SDK over stdio. The MCP tool registry contains only agent operations.

## Checkpoint 5 - Owner Surface and Secrets

Completed. Owner CLI operations, loopback dashboard, CSRF and origin checks, protected token files, the pluggable `SecretProvider` boundary, volatile secret clearing, and no-persistence CVV default are present.

## Checkpoint 6 - Checkout Execution

Completed as an owner-enabled, deny-by-default boundary. The simulator executes known outcomes. The manual provider can run an owner-approved Playwright profile for a policy-validated or explicitly approved intent while a bounded volatile card session is armed. Unknown merchant-controlled forms, hostile redirects, changed checkout facts, missing profiles, unavailable owner authentication, and ambiguous provider outcomes fail closed. Real browser results require independent owner reconciliation; see [docs/checkout-adapters.md](docs/checkout-adapters.md).

## Checkpoint 7 - Adversarial Validation

Completed for the local laboratory. Rust tests, daemon integration, SDK protocol tests, hostile checkout fixture, duplicate and timeout cases, audit tamper detection, owner-boundary tests, and generated-artifact canary scanning are part of `./scripts/verify`.

## Checkpoint 8 - Documentation and Release Readiness

Completed for human review. README, security policy, threat model, research, ADRs, architecture diagrams, KOHO setup, controlled-checkout runbook, owner bootstrap, Codex and Claude Code skill package, incident response, SBOM generation, dependency checks, license checks, and canonical local verification are present. Hosted CI is intentionally omitted at the owner's request for this solo-developer repository. Package publication, account creation, merchant-profile acceptance, and real-money tests remain owner-controlled actions.

## Release Gate

Run `./scripts/verify` from a clean checkout before every push. Before public release, a human must review the exact files listed in the final report, run advisory and static-analysis tools locally, audit dependency licenses, and decide whether a formal payment-security assessment is required.

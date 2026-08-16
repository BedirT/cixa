# Progress

## Current Checkpoint

Checkpoint 8, release-readiness verification.

## Completed

- Rust domain and daemon compile with a locked Cargo graph.
- Integer minor-unit money arithmetic rejects currency mismatch and overflow.
- Deny-by-default policy covers budgets, currencies, merchants, fulfillment, recurring behavior, stored cards, tips, preauthorizations, installments, redirects, attempts, emergency stop, and card-session metadata.
- Agent tokens are scoped, hashed, expiring, revocable records. Owner operations are separated in the request enum and never registered by the MCP server.
- Simulated provider supports approvals, declines, pending holds, refunds, and ambiguous timeouts without automatic retry.
- Ledger distinguishes verified and unverified income. Public receiving instructions explicitly warn that notifications are not settlement evidence.
- HMAC audit chain detects tampering and is stored with a separate key file.
- MCP, TypeScript SDK, Python SDK, CLI, persisted daemon, loopback owner dashboard, hostile merchant fixture, demo, and integration tests exist.
- Documentation and threat model are complete and candid about local-runtime, issuer, merchant, and compliance limits.

## Verification Evidence

The canonical command is `./scripts/verify`. It runs Rust formatting, tests and clippy, TypeScript build and tests, Python tests and compilation, daemon integration, adversarial demo assertions, documentation validation, dependency and license checks, SBOM generation, and generated-artifact secret-canary scanning.

## Security Findings

No unresolved critical or high-severity finding is known within the documented reference threat model. The implementation intentionally fails closed for unsupported browser, manual-provider, issuer-authentication, and ambiguous-network situations instead of claiming universal support.

## Remaining Human Decisions

- Review the exact release diff and lockfiles before publishing.
- Decide whether to install and pin `cargo-audit`, CodeQL, and an independent license scanner in hosted CI.
- Review the KOHO terms and current features again immediately before any real manual test.
- Confirm the owner’s operating-system permissions, local account isolation, browser settings, card limits, fraud-alert process, and emergency-stop runbook.


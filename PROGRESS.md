# Progress

## Current Checkpoint

Checkpoint 10, Docker-first deployment and containerized agent boundary verification.

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
- Manual-provider controlled checkout can preserve a bounded-autonomous policy decision, but executes only with a unique owner-approved merchant profile, hosted payment fields, fresh balance evidence, active agent session, and owner-armed volatile card session.
- The owner console manages KOHO reference metadata, card-session lifetime and operation limits, structured checkout profiles, receiving instructions, reconciliation, and group-shared agent capability files without persisting card data.
- The setup script builds the broker and adapter, installs a private Playwright Chromium, initializes private owner state, and prints matching broker and console commands.
- The Agent Skills-compatible `cixa-payments` package installs for Codex and Claude Code and documents the exact purchase contract, receiving flow, ambiguous-state handling, and no-retry rule.
- Docker Compose is the primary deployment, with separate owner and agent images, fixed UIDs, separate owner and IPC volumes, a loopback-only console, a network-disabled MCP bridge, read-only runtime filesystems, bounded resources, and an end-to-end container gate.
- The README and architecture documentation use a designed SVG system diagram and record the container, storage, IPC, credential, browser, provider, receiving, and reconciliation decisions explicitly.

## Verification Evidence

The canonical commands are `./scripts/verify` and `./scripts/verify-container`. Together they cover Rust formatting, tests and clippy, TypeScript build and tests, Python tests and compilation, daemon integration, one-shot and multi-operation helper flows, responsive owner-console browser checks, adversarial demo assertions, documentation validation, package and skill installation, dependency and license checks, SBOM generation, generated-artifact secret-canary scanning, both image targets, fresh-volume initialization, service health, loopback UI, separate agent UID, and containerized MCP calls.

## Security Findings

No unresolved critical or high-severity finding is known within the documented reference threat model. The implementation intentionally fails closed for unsupported browser, manual-provider, issuer-authentication, and ambiguous-network situations instead of claiming universal support.

## Remaining Human Decisions

- Review the exact release diff and lockfiles before publishing.
- Keep pinned local `cargo-audit` 0.22.2 installed; both Rust lockfiles are mandatory audit inputs. Hosted CI remains intentionally absent for the solo workflow.
- Review the KOHO terms and current features again immediately before any real controlled test.
- Create and verify the dedicated agent OS identity or container, IPC group, shared socket directory, and group-readable capability path before enabling the manual provider.
- Build and test each merchant profile with synthetic checkout data before putting a real card session behind it.
- Confirm the owner’s operating-system permissions, local account isolation, browser settings, card limits, fraud-alert process, and emergency-stop runbook.

# Checkout Adapters

## Simulated Checkout

The local simulator is the canonical supported executor. It uses the same policy and state machine as the daemon, returns deterministic provider outcomes, and never loads a real card or contacts a merchant.

## Supported Owner-Manual Handoff

The shipped real-card handoff sequence is:

1. The agent prepares an intent without payment secrets.
2. The owner runs `begin-handoff`; the broker revalidates policy, reserves funds, and durably records `executing` before returning the complete sanitized checkout facts.
3. The owner suspends the separately identified agent and independently verifies origin, redirect chain, amount, currency, items, fulfillment, recurring indicators, stored-card consent, and form trust in an owner-only browser.
4. The owner enters payment material directly into that browser and submits at most once. The broker and agent never receive it.
5. The owner runs `complete-handoff`, which records `unknown` with retry disabled even when the merchant page claims success.
6. The owner verifies the provider transaction and uses `reconcile`; only then can the broker settle and issue a sanitized receipt.

The split commands are crash-safe: a restart after `begin-handoff` recovers durable `executing` state as `unknown`, so the operation cannot return to an executable approval. If any property is unknown, the owner must not submit. The broker never relies on the agent's natural-language summary of a page.

## Embedded Automated Handoff Boundary

The Rust domain also exposes `owner_execute_approved_handoff_persisted` for a separately reviewed trusted adapter. It persists `executing` before invoking external submission, consumes a bound secret once, runs cleanup on secret-fetch and submission failures, sanitizes the outcome, and persists terminal or quarantined state before returning. It is not exposed to agent RPC and is not a claim that this repository independently observes an arbitrary merchant DOM. An adapter that cannot enforce the full critical-section contract below must not call it.

Signed helper grants are bound to a helper instance and broker UID, expire after five minutes, and require `DurableNonceRedemptionStore` for atomic create-once redemption across workers and restarts. A helper must obtain the broker UID from Unix peer credentials rather than trusting request data.

## Hosted Fields and Merchant Trust

The policy distinguishes recognized hosted fields, an explicitly owner-approved merchant integration, and an unknown merchant-controlled payment form. The latter is approval-required in the default policy. In the simulator, these values are synthetic adversarial fixtures. For a manual card, every checkout requires owner approval and ends in reconciliation, so agent-asserted origin, amount, or form trust can never cause autonomous real spending. A future real executor must independently validate HTTPS origin, canonical host, resolved address, redirect chain, final visible total, currency, purchase type, and form ownership before authorization.

## Browser Reference Boundary

The repository includes a hostile static merchant laboratory and the `CheckoutExecutor` trait. It does not ship a universally safe Playwright payment implementation. A browser automation implementation that cannot prevent agent DOM reads, autofill reads, screenshot and trace capture, clipboard access, network-body recording, or remote debugging must return an explicit unsupported result. This safe fallback is preferable to a superficially complete but unsafe automation path.

## Failure Behavior

No blind refresh, form resubmission, or automatic retry follows a timeout. `provider_pending`, `unknown`, and `reconciliation_required` are owner workflows. A misleading success page is not provider evidence. A provider reference or owner-authenticated reconciliation is required before a settled ledger event is recorded.

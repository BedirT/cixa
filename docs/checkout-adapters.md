# Checkout Adapters

## Simulated Checkout

The local simulator is the canonical supported executor. It uses the same policy and state machine as the daemon, returns deterministic provider outcomes, and never loads a real card or contacts a merchant.

## Secure Handoff

The intended handoff sequence is:

1. The agent prepares an intent without payment secrets.
2. The broker acquires exclusive checkout control and pauses agent browser access.
3. The broker validates final origin, redirect chain, amount, currency, fulfillment, recurring indicators, stored-card consent, and form trust.
4. The broker revalidates policy and reserves funds transactionally.
5. A just-in-time owner-controlled secret provider supplies volatile payment material.
6. The trusted executor submits exactly once and observes the result.
7. The broker clears secret material, sanitizes or destroys the context, appends the ledger and audit entries, and returns only a sanitized result.

If any property is unknown, the broker requires owner approval or denies. It never relies on the agent's natural-language summary of a page.

## Hosted Fields and Merchant Trust

The policy distinguishes recognized hosted fields, an explicitly owner-approved merchant integration, and an unknown merchant-controlled payment form. The latter is approval-required in the default policy. The broker validates HTTPS origin, canonical host, no credentials or explicit ports, no local/private/link-local/metadata destination, redirect count, and final visible total.

## Browser Reference Boundary

The repository includes a hostile static merchant laboratory and the `CheckoutExecutor` trait. It does not ship a universally safe Playwright payment implementation. A browser automation implementation that cannot prevent agent DOM reads, autofill reads, screenshot and trace capture, clipboard access, network-body recording, or remote debugging must return an explicit unsupported result. This safe fallback is preferable to a superficially complete but unsafe automation path.

## Failure Behavior

No blind refresh, form resubmission, or automatic retry follows a timeout. `provider_pending`, `unknown`, and `reconciliation_required` are owner workflows. A misleading success page is not provider evidence. A provider reference or owner-authenticated reconciliation is required before a settled ledger event is recorded.


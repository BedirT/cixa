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

## Experimental Controlled Browser Handoff

The repository ships `packages/checkout-playwright` plus the owner-only `execute-handoff` command for a separately reviewed merchant integration. `execute-handoff` takes the exclusive broker data lock, so a running daemon must stop first and the agent loses RPC access before the browser starts. It persists `executing`, obtains a signed one-shot helper grant, launches a fresh browser process and context without downloads, service workers, permissions, tracing, video, screenshots, or a remote debugging endpoint, and destroys that context before returning.

The adapter permits only an exact approved HTTPS navigation chain and owner-configured hosted-field processor origins that are disjoint from every merchant navigation origin. It rejects private or reserved network destinations, non-hosted payment forms, changed visible totals, currencies, line items, fulfillment profiles, recurring, trial, or stored-card indicators, and missing sanitized provider references. It rechecks all facts and frame ownership immediately before the single submit. Payment fields are filled only in a cross-origin iframe whose live origin is explicitly configured. The adapter configuration, browser and Node executables, and adapter script must be absolute, owner-controlled, regular non-symlink files that are not group or world writable.

This is not a generic DOM heuristic. Each merchant requires an owner-reviewed selector configuration, exact trusted origins, and a browser executable path. Any unavailable or ambiguous evidence fails before submission or produces `unknown` with retry disabled. Test the integration with synthetic credentials before considering a deliberately low-balance card.

The adapter config is a private JSON file with `browserExecutable`, `checkoutUrl`, `allowedNavigationOrigins`, `allowedProcessorOrigins`, `timeoutMs`, and selectors for `finalTotal`, `currency`, `items`, `fulfillment`, the three consent indicators, the hosted payment iframe and its fields, submit control, outcome markers, and provider reference. The `items` element must expose canonical JSON matching the approved line-item array. Selector configuration is code-like trust policy: review and protect it as owner-controlled configuration.

Initialize helper key material with `treasury init-helper --helper-dir DIR`. Start the one-operation helper with `treasury secret-helper`, piping one JSON secret object on stdin from an owner-only terminal or credential-store bridge. It authenticates the broker through Unix peer credentials, atomically redeems the signed grant across workers and restarts, returns at most 4096 bytes once, zeroes its buffer, removes its socket, and exits. Grants are bound to the helper instance and broker UID and expire after five minutes. Rotate by replacing the private helper directory only when no handoff is active.

Run `execute-handoff` with absolute paths for Node, `packages/checkout-playwright/dist/index.js`, and the private adapter config, plus the helper socket, key, and ID files. Stop `treasury serve` first; failure to acquire the exclusive data lock prevents browser launch. Payment material is accepted only by the helper on stdin, never in arguments, config, environment variables, output, or broker state.

`tests/handoff.py` exercises the full helper and persisted orchestration protocol. Browser-independent tests cover fail-closed configuration and money and origin binding. A real browser run remains merchant-specific and must use a synthetic test checkout because project verification never contacts a real merchant or uses a real card.

The Rust broker owns a hard wall-clock deadline in addition to Playwright operation timeouts. On expiry it kills and reaps the adapter process group and persists the intent as `unknown`; the integration test exercises a non-responsive adapter. Rust protocol buffers containing the copied secret are explicitly zeroized after the pipe write. JavaScript strings cannot be reliably zeroized, so the short-lived adapter process exits after one request and OS process teardown is the residual-memory boundary.

## Hosted Fields and Merchant Trust

The policy distinguishes recognized hosted fields, an explicitly owner-approved merchant integration, and an unknown merchant-controlled payment form. The latter is approval-required in the default policy. In the simulator, these values are synthetic adversarial fixtures. For a manual card, every checkout requires owner approval, so agent-asserted origin, amount, or form trust can never cause autonomous real spending. The controlled adapter independently validates HTTPS origin, public network destination, redirect chain, visible total, currency, line items, fulfillment, tip, preauthorization, installments, consent facts, and cross-origin form ownership before submission. Each evidence selector must resolve to exactly one visible element.

## Browser Reference Boundary

The agent never receives Playwright, CDP, DOM, autofill, clipboard, screenshot, trace, video, console, or network-body access to the payment-critical process. The adapter uses a fresh process and context and does not expose those APIs. An integration that needs capture channels, a merchant-controlled card field, arbitrary navigation, or a shared agent browser must return an explicit unsupported result.

## Failure Behavior

No blind refresh, form resubmission, or automatic retry follows a timeout. Browser and merchant DOM output always maps to `unknown`, even if the page displays success, pending, decline, or a reference. Only owner-authenticated reconciliation using independent issuer or processor evidence can settle or definitively decline the intent.

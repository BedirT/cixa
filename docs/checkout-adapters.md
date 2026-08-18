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

## Controlled Browser Checkout

The repository ships `packages/checkout-playwright` for separately reviewed merchant integrations. There are two entry points:

- `execute-handoff` is an owner-only, one-intent maintenance flow. It takes the exclusive broker data lock, so the daemon must be stopped.
- `cixa serve` accepts `--checkout-runtime-dir`, `--checkout-profiles-dir`, `--node-path`, and `--adapter-script`. When the owner enables controlled checkout, an agent with a policy-validated or explicitly approved intent may execute through that configured runtime.

Both paths persist `executing` before browser submission, obtain a signed intent-bound helper grant, launch a fresh browser process and context without downloads, service workers, permissions, tracing, video, screenshots, or a remote debugging endpoint, and destroy that context before returning. The daemon serializes treasury state while a checkout is in its critical section.

The adapter permits only an exact approved HTTPS navigation chain and owner-configured hosted-field processor origins that are disjoint from every merchant navigation origin. It rejects private or reserved network destinations, non-hosted payment forms, and changed visible totals, currencies, line items, fulfillment profiles, recurring, trial, stored-card, tip, preauthorization, or installment indicators. It rechecks all facts and frame ownership immediately before the single submit. Payment fields are filled only in a cross-origin iframe whose live origin is explicitly configured. Browser HTTP requests are routed through the origin and public-address checks, unlisted hostnames fail resolution, all WebSockets are closed without connecting, and WebRTC and WebTransport constructors are disabled before page script. Integrations that require those channels are unsupported. The adapter configuration, browser and Node executables, and adapter script must be absolute, owner-controlled, regular non-symlink files beneath root- or owner-controlled non-writable directory chains. The owner UID is part of the trusted computing base: an attacker that can replace those files as that UID is outside the local threat model, so path validation is not presented as protection from a same-UID race.

This is not a generic DOM heuristic. Each merchant requires an owner-reviewed selector configuration, exact trusted origins, and a browser executable path. Any unavailable or ambiguous evidence fails before submission or produces `unknown` with retry disabled. Test the integration with synthetic credentials before considering a deliberately low-balance card.

The adapter config is a private JSON file with `browserExecutable`, `checkoutUrl`, `allowedNavigationOrigins`, `allowedProcessorOrigins`, `timeoutMs`, and selectors for all approved checkout facts, the hosted payment iframe and its fields, and the submit control. The broker validates and parses this file once, then sends that immutable parsed value to the short-lived adapter instead of allowing a second path-based read. The `items` element must expose canonical JSON matching the approved line-item array. Selector configuration is code-like trust policy: review and protect it as owner-controlled configuration.

Initialize helper key material with `cixa init-helper --helper-dir DIR`. `cixa secret-helper` is the one-operation owner handoff. `cixa secret-session` accepts the same strict JSON secret over stdin and serves a bounded number of signed operations until its short TTL or operation count expires. The owner console manages `secret-session`, never stores the input, clears the browser form, shows the expiry, and can terminate it immediately.

The socket must use an absolute path beneath a root- or owner-controlled non-writable directory chain. The helper authenticates the broker through Unix peer credentials, and the broker independently requires the connected helper peer to use the owner UID before sending the grant. Each grant is atomically and durably redeemed, bound to one intent, helper instance, broker UID, and expiry, and returns at most 4096 bytes. The volatile secret buffer is zeroed on exit. Rotate helper key material only when no checkout session is active.

Run `execute-handoff` with absolute paths for Node, `packages/checkout-playwright/dist/index.js`, and the private adapter config, plus the helper socket, key, and ID files. Stop `cixa serve` first; failure to acquire the exclusive data lock prevents browser launch. For daemon checkout, the owner console writes structured, permission-checked profiles under the configured profile directory, and the broker loads the unique profile matching the canonical merchant. Payment material is accepted only by the helper on stdin, never in arguments, profile, environment variables, output, or broker state.

`tests/handoff.py` exercises the one-shot and multi-operation helper protocols and persisted orchestration. Domain tests cover policy-authorized controlled checkout, fail-closed state transitions, and secret non-persistence. Browser-independent tests cover configuration, money, origin, network, and live fact binding. A real browser run remains merchant-specific and must use a synthetic test checkout because project verification never contacts a real merchant or uses a real card.

The Rust broker owns a hard wall-clock deadline in addition to Playwright operation timeouts. The deadline covers helper connection and I/O, the adapter parent process, and stdout drainage. On adapter expiry the broker kills and reaps its process group, tracks observed descendants for explicit termination, drops its pipe endpoint, and persists the intent as `unknown`. The root remains unreaped until group cleanup, and descendant signaling uses Linux pidfds or macOS audit tokens so a recycled PID cannot target another process. Integration tests exercise both a non-responsive adapter and a detached descendant retaining stdout. This contains the shipped, owner-reviewed adapter and ordinary browser process tree; it is not a sandbox for a malicious owner executable that deliberately escapes its process group between observations. The helper connection worker belongs to the one-shot `execute-handoff` process, so process exit is its final deadline boundary even if an operating-system connect call does not return. The helper and signing-key buffers use zeroizing wrappers, and the broker accepts only one strict, size-bounded secret JSON object before structurally serializing it into a zeroizing wire buffer. JavaScript strings cannot be reliably zeroized, so the short-lived adapter process exits after one request and OS process teardown is the residual-memory boundary.

## Hosted Fields and Merchant Trust

The policy distinguishes recognized hosted fields, an explicitly owner-approved merchant integration, and an unknown merchant-controlled payment form. The latter is approval-required in the default policy. In the simulator, these values are synthetic adversarial fixtures. With controlled checkout disabled, a manual card always requires owner approval. With it enabled, agent assertions still cannot authorize a charge by themselves: the controlled adapter independently validates HTTPS origin, public network destination, redirect chain, visible total, currency, line items, fulfillment, tip, preauthorization, installments, consent facts, and cross-origin form ownership before submission. Each evidence selector must resolve to exactly one visible element.

## Browser Reference Boundary

The agent never receives Playwright, CDP, DOM, autofill, clipboard, screenshot, trace, video, console, or network-body access to the payment-critical process. The adapter uses a fresh process and context and does not expose those APIs. An integration that needs capture channels, a merchant-controlled card field, arbitrary navigation, a CAPTCHA, owner authentication, 3-D Secure interaction, or a shared agent browser must return an explicit unsupported result.

## Failure Behavior

No blind refresh, form resubmission, or automatic retry follows a timeout. Browser and merchant DOM output always maps to `unknown`, even if the page displays success, pending, decline, or a reference. Only owner-authenticated reconciliation using independent issuer or processor evidence can settle or definitively decline the intent.

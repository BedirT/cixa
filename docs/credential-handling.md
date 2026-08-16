# Credential Handling

## Protected Assets

PAN, expiry, CVV/CVC, billing and shipping identity, phone number, account aliases, owner authentication material, agent capability tokens, and audit integrity keys are sensitive. They must not appear in command-line arguments, source fixtures, ordinary environment variables, logs, traces, screenshots, crash artifacts, MCP output, or API errors.

## Secret Provider Modes

The core exposes a one-shot, operation-bound `SecretProvider` trait with these implemented modes:

1. `InteractiveOwnerEntryProvider`: the owner supplies bytes over a caller-controlled reader for one approved operation; the provider consumes them once.
2. `VolatileSessionSecretProvider`: the daemon keeps only transaction-scoped bytes, consumes them once, and best-effort zeroizes them on drop.
3. `OwnerControlledSecretHelperProvider` on Unix: a private local socket receives only the bound operation ID, approved intent ID, and card reference, then returns a length-bounded secret. The broker rejects non-socket or group/world-accessible endpoints.
4. Simulated test provider: synthetic canaries are used only in local tests.

`Treasury::bind_approved_secret_operation` requires the owner capability, an intent already in the explicit `approved` state, and the configured manual card reference. Every provider compares that complete binding before retrieval. None of these providers is exposed through the agent RPC or MCP surface.

The `ManualPrepaidCardProvider` stores a `SecretReference`, provider kind, masked last four, and a freshness-labeled balance snapshot. It does not store a PAN or CVV. The optional OS keychain design remains explicit owner opt-in work. macOS Keychain, Linux Secret Service, and Windows Credential Manager each have different UI, session, backup, and access-control semantics.

## CVV

PCI SSC guidance says card verification codes are sensitive authentication data and must not be stored after authorization, even encrypted. This project therefore does not persist CVV in state, browser profiles, config, logs, or test fixtures and does not claim PCI compliance. Encryption at rest would not make post-authorization CVV storage automatically compliant.

## Browser Exposure

The agent must never receive CDP, WebDriver, Playwright, DOM, autofill, clipboard, screenshot, trace, video, console, or network-body access to a payment-critical browser. A future executor must use an ephemeral profile, suspend agent control before secret entry, disable capture, and destroy the profile. The current reference safely denies unsupported browser handoff rather than implying those properties exist.

## Redaction and Residual Risk

Receipts contain merchant domain, amount, state, provider reference, and a redaction marker, not contact or payment data. The redactor has synthetic canary tests, but no language runtime can guarantee memory zeroization. A compromised administrator or kernel can bypass process boundaries. Keep the issuer's card balance deliberately small and manually lock or replace the card after a risky run.

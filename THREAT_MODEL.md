# Threat Model

## Scope and Method

This model uses STRIDE categories plus payment-specific abuse cases. The protected asset is the owner's bounded financial authority and the confidentiality of payment and owner credentials. The broker is trusted application code; the agent, merchant content, model provider, notifications, browser content, and provider output are untrusted inputs.

## Assets and Trust Boundaries

| Asset | Boundary | Required property |
| --- | --- | --- |
| PAN, expiry, CVV, owner identity | Secret provider to checkout executor | Confidentiality, just-in-time use, no persistence of CVV |
| Agent capability token | Owner provisioning to agent IPC | Narrow scope, expiry, revocation, no owner operations |
| Owner token and audit key | Owner filesystem to daemon | Private permissions, never agent-readable, rotate on incident |
| Policy and limits | Owner to broker | Authenticated, versioned, append-only audit evidence |
| Budget reservations | Agent request to broker/provider | Atomic, no negative authority, no concurrent double spend |
| Ledger and transaction state | Broker storage to provider | Crash-safe, explicit ambiguity, tamper evidence |
| Receiving instructions | Owner to agent | Public approved data only, no login identity or secret |

## Controls by Threat

| Threat and STRIDE | Preventive controls | Detective controls | Recovery controls |
| --- | --- | --- | --- |
| Prompt injection in merchant copy or tool output, spoofing/tampering | Merchant content is data; typed policy outside the agent; owner approval is a real owner operation | Audit decision reasons, hostile fixture, strict schemas | Deny or queue; revoke agent and emergency stop |
| Compromised agent process, elevation of privilege | Scoped hashed token, expiry, revocation, no owner tools, no policy mutation | Owner audit and capability inspection | Revoke token, stop daemon, rotate token |
| Malicious merchant page or browser JavaScript, information disclosure | Hosted-field preference, merchant trust tiers, agent suspension during critical section, no direct CDP | Hostile merchant scenarios, canary scan | Deny unsupported form; lock/replace card manually |
| Secret exfiltration, model-provider logging | SecretProvider boundary, no credentials in MCP, volatile secret, redacted receipt, no telemetry | Generated-artifact canary scan and gitleaks in CI | Treat secret as compromised, rotate/replace card, destroy browser artifacts |
| Screenshots, traces, videos, DOM snapshots, console or network-body capture | No payment browser exposed to agent; future executor must disable capture and use ephemeral profile | Artifact scan and executor review | Destroy profile and artifacts; manual issuer action |
| Local unprivileged process, socket theft, token theft | Private state and socket permissions, token file, local-only default | Audit/auth failures and file-permission checks | Revoke token, remove socket, rotate credentials |
| Replay, duplicate idempotency, concurrent double spend | Caller idempotency, one active execution, serialized daemon, reservations | Provider charge count, ledger duplicate assertions | Reconcile only once; quarantine unknown |
| CSRF, XSS, owner-interface takeover | Loopback bind, Host and Origin checks, POST-only mutation, synchronizer token, CSP, no third-party assets | Dashboard integration and security headers | Stop daemon, inspect audit, rotate owner token |
| SSRF, DNS rebinding, Unicode/homograph and redirect manipulation | HTTPS-only canonicalization, IDNA ASCII, no credentials/ports, no local/private/link-local/metadata IPs, redirect limit, revalidation contract | URL unit tests and hostile fixture | Deny checkout; do not retry |
| Amount or currency substitution, hidden subscription, preauthorization, stored-card consent | Integer money, final-total recheck, currency allowlist, deny recurring/trials/tips/holds/stored card by default | Decision evidence and adversarial cases | Approval or denial; owner reconciles provider |
| Process crash or network loss after submit | Explicit `unknown`, no blind retry, state persistence, provider reference capture | Restart test and reconciliation state | Owner verifies issuer and marks settled/declined/refunded |
| Ledger tampering or audit deletion | Atomic state write, append-only event model, separate HMAC key, hash-chain verification | Load-time and owner export chain check | Preserve sanitized evidence; rotate key and restore trusted backup |
| Forged income, spoofed email, screenshot, webhook | Only official adapter, owner-authenticated action, or configured signed integration can verify income | Verified/unverified status and ledger source | Keep unverified notification non-spendable; reconcile manually |
| Provider fraud controls, 3-D Secure, CAPTCHA, identity checks bypass | No issuer login automation or bypass; manual provider returns unknown/manual boundary | Provider decline and alert review | Owner handles official app, lock, or support process |
| Malicious dependency or package-install script | Lockfiles, npm audit, Cargo metadata, optional cargo-audit, license check, CI and CodeQL workflow | SBOM and dependency checks | Pin/upgrade, revoke builds, investigate release artifacts |
| Unsafe update or accidental public exposure | No public bind, explicit TCP not implemented by default, release review, versioned state | Socket permissions, docs and CI checks | Stop daemon, rotate credentials, restore reviewed build |

## Out of Scope and Assumptions

- A compromised kernel, root/administrator, hypervisor, or physically hostile device can read or alter local state and memory.
- The card issuer, payment processor, or merchant may be compromised, dishonest, unavailable, or inconsistent.
- The owner may be dishonest or intentionally configure unsafe limits; this project cannot protect an owner from the owner.
- Chargebacks, disputes, tax, accounting, financial advice, and legal approval in every jurisdiction are out of scope.
- Universal merchant compatibility and universal browser automation are out of scope.
- Formal PCI certification or compliance is out of scope. The implementation follows a conservative no-CVV-persistence posture but is not an assessment.
- The model provider may retain prompts or tool results. The MCP surface must not send secrets, but the agent's broader environment remains an owner responsibility.
- OS filesystem permissions are assumed to work against ordinary unprivileged local processes.

## Residual Risk Decision

No critical or high-severity finding is left unresolved within this documented reference threat model. Unsupported or uncertain paths fail closed, become approval-required, or become owner reconciliation tasks. The residual local-admin, issuer, browser-runtime, dependency, and legal risks above require human controls and are not silently represented as solved by a green test suite.


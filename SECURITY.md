# Security Policy

## Scope

This project is a local payment authorization gateway and policy firewall for untrusted software agents. The supported reference release is `0.1.x`. It is not a bank, wallet, issuer, payment processor, or claim of PCI DSS compliance.

## Supported Versions

| Version | Supported |
| --- | --- |
| 0.1.x | Security fixes and review |

Because this project controls access to payment credentials, do not assume an old data directory is safe after an upgrade. Stop the broker, back up only sanitized records, review the release notes, and migrate through a documented versioned procedure.

## Responsible Disclosure

Do not open a public issue containing credentials, full PAN, CVV/CVC, recovery information, private KOHO data, screenshots, browser traces, or a live exploit against a real account. Prefer GitHub's private vulnerability reporting for the repository, or contact the maintainers through a verified private channel before publication. If no private channel is configured, report only a minimal reproduction without sensitive material and request a private follow-up.

Include the affected version, platform, exact command, sanitized logs, threat-model boundary, impact, and a proposed mitigation. Synthetic canaries are preferred. Never include real payment credentials, owner tokens, agent tokens, or raw audit keys in a report.

## Secret Handling

- PAN, expiry, CVV/CVC, billing identity, shipping identity, owner authentication, capability tokens, and audit keys are sensitive.
- Secrets must not be put in source, shell arguments, ordinary environment variables, logs, traces, screenshots, crash reports, or MCP output.
- The default provider stores references and masked last-four metadata only. CVV is volatile and best-effort cleared after an operation.
- A compromised OS administrator or kernel can bypass local process boundaries. An external append-only sink is required for stronger tamper evidence.

## Incident Basics

1. Trigger the owner emergency stop and stop the daemon.
2. Lock the card in the issuer's official application and follow the issuer's fraud process.
3. Revoke the affected agent token and rotate owner and audit credentials through an owner-controlled procedure.
4. Preserve sanitized audit data and the exact binary, lockfiles, and hashes.
5. Search logs, traces, screenshots, crash artifacts, MCP output, and browser profiles for canary or real secrets.
6. Reconcile every unknown transaction with the issuer before any new execution.

See [docs/incident-response.md](docs/incident-response.md) for the detailed runbook.


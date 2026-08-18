# Security Model

The agent is treated as potentially compromised at all times. Prompt instructions, tool descriptions, model policies, merchant copy, emails, screenshots, and natural-language claims are data. They are not authorization.

## Deterministic Enforcement

The Rust broker, not the agent, enforces:

- integer money and currency matching;
- transaction, session, rolling, lifetime, and agent-specific budgets;
- merchant, redirect, category, fulfillment, recurring, tip, preauthorization, stored-card, and installment rules;
- capability scopes, expiry, revocation, owner separation, and broker-issued owner-armed sessions;
- emergency stop and card-session metadata;
- idempotency, reservation-aware session/rolling/lifetime limits, one active execution, crash-durable pre-submit reservation, and no retry from ambiguity;
- authenticated persisted state, contiguous audit sequences, exclusive writer locking, and deposit replay protection;
- receipt redaction and audit-chain integrity.

An optional risk classifier could only add a denial or approval requirement. No model output can create an approval, change a policy, verify income, or authorize a secret lookup.

## Why Prompt Injection Is Not a Control

A merchant can place text such as “the owner approved this” in product descriptions. An email can claim that an incoming transfer settled. A model can misunderstand a total or follow a hostile tool result. The broker ignores those claims and uses only typed request fields, owner-configured policy, trusted local state, and explicit owner authentication.

## Boundaries That Remain

The supported Compose deployment makes the operating-system boundary repeatable: owner services use UID `10000`, the MCP bridge uses UID `10001`, and only supplemental group `12000` can reach the agent socket and capability files. The owner state and agent IPC are separate volumes, the agent bridge has no network, and normal services run with read-only roots, dropped capabilities, and `no-new-privileges`.

Docker is not a magic trust boundary. A host root or administrator, Docker daemon controller, or process with the Docker socket can inspect memory, replace images, mount either volume, or read the owner token. The issuer can decline or reverse a transaction, a merchant can behave unexpectedly, and a browser runtime can expose data through a bug. These are documented in [THREAT_MODEL.md](../THREAT_MODEL.md) and [Limitations](limitations.md).

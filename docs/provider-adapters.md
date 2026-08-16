# Provider Adapters

## Interface

`PaymentProvider` is intentionally small:

- `provider_id()` identifies the adapter;
- `available_balance()` returns a typed balance or an explicit reconciliation error;
- `authorize(intent)` returns approved, declined, pending, or unknown.

The core never converts a provider balance into permission without applying the stricter owner policy and active reservations. Provider references are stored with the intent, not exposed as credentials.

## Simulated Provider

`SimulatedProvider` is deterministic and persisted with local demo state. It supports:

- a starting balance;
- available balance after holds;
- approved, declined, pending, and ambiguous outcomes;
- provider references;
- exactly-once charge storage by intent ID;
- delayed settlement and refund paths;
- incoming-deposit storage for tests.

It is not a production payment processor and never reaches the network.

## Manual Prepaid Card

`ManualPrepaidCardProvider` is the first real-world adapter shape. It carries a `SecretReference` and an owner-confirmed balance snapshot with `estimated`, `owner_confirmed`, or `provider_verified` status plus freshness metadata. It reports manual checkout as an ambiguous boundary rather than pretending the broker submitted a real charge.

For a KOHO card, the owner performs the checkout or uses a separately reviewed handoff, confirms the issuer result in the official app, and then reconciles the intent. The adapter does not log in, scrape, call private endpoints, alter card locks, approve fraud alerts, or replace a card.

Verified income is recorded separately from starting capital. An owner may attach a verified deposit to an agent policy with an integer `reinvestment_ratio_bps` from 0 to 10,000. Only the reinvested amount becomes policy authority; the policy also enforces an absolute exposure ceiling and maximum treasury size. An email or screenshot remains unverified and contributes no reinvestment authority.

## Future Official Provider

A future official adapter must prove authenticated balance and transaction references from public supported documentation, preserve idempotency, distinguish pending from settled, and document rate limits, reconciliation, and data retention. It must not silently broaden capabilities or treat an email, screenshot, or agent statement as verified income.

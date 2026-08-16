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

`ManualPrepaidCardProvider` is the first real-world adapter shape. It carries a `SecretReference` and a balance snapshot labeled `estimated` or `owner_confirmed` with freshness metadata. Manual owner input cannot claim `provider_verified`; that status is reserved for a future authenticated official or signed import path. It reports manual checkout as an ambiguous boundary rather than pretending the broker submitted a real charge.

For a KOHO card, the owner performs the checkout or uses a separately reviewed handoff, confirms the issuer result in the official app, and then reconciles the intent. The adapter does not log in, scrape, call private endpoints, alter card locks, approve fraud alerts, or replace a card.

Runtime configuration uses `treasury configure-manual-provider` with a credential-helper reference, last four digits, typed balance, verification status, and freshness TTL. The command rejects apparent payment data in the reference. Estimated and expired snapshots remain visible as labeled information but cannot authorize spending. Manual execution always requires an authenticated owner approval and never claims that the broker submitted the card.

Verified income is recorded separately from starting capital. An owner must attach a verified deposit to an explicit agent policy and a unique external transaction reference. Replays of the same reference are idempotent, while conflicting reuse is rejected. The policy applies an integer `reinvestment_ratio_bps` from 0 to 10,000. Only the reinvested amount becomes policy authority; the policy also enforces an absolute exposure ceiling and maximum treasury size. An email or screenshot remains unverified and contributes no reinvestment authority.

## Future Official Provider

A future official adapter must prove authenticated balance and transaction references from public supported documentation, preserve idempotency, distinguish pending from settled, and document rate limits, reconciliation, and data retention. It must not silently broaden capabilities or treat an email, screenshot, or agent statement as verified income.

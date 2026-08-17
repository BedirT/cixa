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

`ManualPrepaidCardProvider` is the first real-world adapter shape. It carries a `SecretReference` and a balance snapshot labeled `estimated` or `owner_confirmed` with freshness metadata. Manual owner input cannot claim `provider_verified`; that status is reserved for a future authenticated official or signed import path.

For a KOHO card, the owner may perform the checkout manually or explicitly enable controlled checkout. Controlled checkout still uses the manual provider: it submits through an owner-approved hosted-fields profile while an owner-armed volatile card session is active, then returns an ambiguous result until the owner confirms the issuer record in KOHO. The adapter does not log in, scrape, call private endpoints, alter card locks, approve fraud alerts, or replace a card.

Runtime configuration uses `cixa configure-manual-provider` or the owner console with a credential-helper reference, last four digits, typed balance, verification status, freshness TTL, and explicit controlled-checkout flag. The command rejects apparent payment data in the reference. Estimated and expired snapshots remain visible as labeled information but cannot authorize spending. With controlled checkout disabled, every real purchase requires authenticated owner approval and handoff. With it enabled, the broker preserves the agent policy decision but still requires a matching owner profile, an active agent session, a live owner payment session, hosted fields, and every ordinary budget and merchant check.

The durable provider record never contains the PAN, expiry, CVV, cardholder, or KOHO login. The owner console sends card material only to `cixa secret-session`, which holds one strict JSON object in process memory for at most 60 minutes and 100 signed operations, with shorter defaults in the UI. Each broker retrieval is bound to one intent, helper instance, broker UID, expiry, and durable one-time grant redemption.

Verified income is recorded separately from starting capital. An owner must attach a verified deposit to an explicit agent policy and a unique external transaction reference. Replays of the same reference are idempotent, while conflicting reuse is rejected. The policy applies an integer `reinvestment_ratio_bps` from 0 to 10,000. Only the reinvested amount becomes policy authority; the policy also enforces an absolute exposure ceiling and maximum treasury size. An email or screenshot remains unverified and contributes no reinvestment authority.

## Future Official Provider

A future official adapter must prove authenticated balance and transaction references from public supported documentation, preserve idempotency, distinguish pending from settled, and document rate limits, reconciliation, and data retention. It must not silently broaden capabilities or treat an email, screenshot, or agent statement as verified income.

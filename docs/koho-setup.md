# Using Cixa With KOHO

Last reviewed: 2026-08-17. KOHO features, fees, limits, product availability, and terms can change. Cixa is independent of KOHO, Mastercard, and Interac. This is a technical setup guide, not financial, legal, or compliance advice.

## The boundary

Cixa treats KOHO as a user-owned prepaid card and a source of owner-checked evidence. It does not log in to KOHO, scrape the app, call a private API, read one-time codes, change card controls, approve fraud alerts, or send Interac e-Transfers.

The agent never gets the KOHO password, recovery details, full card number, expiry, CVV, owner token, dashboard session, or payment helper. When you arm a payment session, the card is passed from the local owner form directly to a short-lived helper process. It is not written to Cixa state, profiles, audit, logs, MCP output, or receipts.

## Prepare the KOHO side

1. Create and verify your KOHO account using KOHO's official app or website and review the current [KOHO legal terms](https://www.koho.ca/legal/).
2. Turn on two-step verification. KOHO documents an authenticator option in its [two-step verification guide](https://help.koho.ca/en_us/set-up-two-step-verification-HkZqqo9IZx).
3. Prefer a dedicated virtual card and keep only a deliberately small balance exposed. KOHO explains its virtual and physical cards in its [card guide](https://help.koho.ca/en_us/whats-the-difference-between-a-virtual-card-and-a-physical-card-B1DFS2vEGv).
4. Enable useful KOHO transaction alerts and know how to lock the card. Keep borrowing, overdraft, pay-later, cash-advance, credit-building, crypto, and similar features outside the Cixa card setup.
5. Run the agent under a distinct OS identity or container as described in [deployment.md](deployment.md). An unrestricted agent under your owner login could read whatever your login can read.

## Prepare Cixa

From the repository:

```bash
./scripts/cixa-docker up
./scripts/cixa-docker dashboard-token
```

Open `http://127.0.0.1:8765`, unlock the console, and go to **Trust → Provider**. The supplied Compose stack already runs the broker and MCP bridge under different UIDs and keeps the owner and agent volumes separate. Do not replace those mounts with one shared data directory.

### 1. Connect the manual provider

Enter:

- an owner-facing reference such as `keychain://cixa/koho-card`;
- where you keep that credential reference;
- the card's last four digits;
- the available balance and currency you just checked in KOHO;
- how long that balance evidence should remain fresh.

The reference is a label, not a place for card data. Cixa rejects references that look like payment credentials. An estimated or expired balance is still visible, but cannot authorize a purchase.

Enable **controlled checkout** only after the agent boundary and merchant profiles are ready. After switching from the simulator to a manual provider, same-UID agent connections are rejected immediately, even if the daemon was already running.

### 2. Approve merchant profiles

Each real merchant needs one owner-reviewed profile. The profile contains:

- the canonical merchant domain;
- every HTTPS merchant origin in the exact redirect chain;
- the separate hosted-fields payment processor origins;
- the browser executable and timeout;
- selectors for total, currency, items, fulfillment, recurring/trial/stored-card consent, tip, preauthorization, and installments;
- selectors for the cross-origin payment iframe, card fields, and submit button.

Cixa accepts only recognized cross-origin hosted fields. A merchant-controlled card input, missing evidence, multiple visible matches, private network destination, surprise origin, changed total, or changed consent fact fails closed. Profiles are owner-only policy, not something an agent may create.

Test each integration with the merchant's sandbox or synthetic card flow first. Merchant markup changes can make a profile stop working, which is safer than silently guessing.

### 3. Open a payment session

In **Trust → Provider**, enter the card number, expiry, CVV, and cardholder in the local payment-session form. Choose a short expiry and a small maximum number of checkouts. Arm it only while you expect the agent to buy.

The browser form clears after submission. The helper keeps the structured secret only in volatile process memory and exits after its time or operation limit. You can end it immediately from the same screen. JavaScript and ordinary operating systems cannot promise perfect memory zeroization, so process isolation, short lifetime, and a low card balance are the practical residual-risk boundary.

### 4. Set agent authority

In **Agents**, keep each capability scoped and short-lived. Set per-purchase, session, rolling 24-hour, lifetime, and absolute exposure limits. Approve only the merchants and fulfillment profiles the agent actually needs. `approval_required` is a good first mode; move to `bounded_autonomous` only after a synthetic end-to-end run.

## What happens during a payment

1. The agent reads its budget and prepares exact final checkout facts through MCP or an SDK.
2. Cixa evaluates policy and either denies, waits for you, or marks the intent policy-validated.
3. On a single execute call, the broker loads the matching owner profile, binds a signed one-operation grant, and asks the helper for the card.
4. A fresh isolated browser validates the approved redirect chain and visible checkout facts, fills only the approved processor iframe, revalidates everything, and submits once.
5. Cixa records the result as `unknown` or `reconciliation_required`, never as settled based on merchant page text.
6. You check the KOHO transaction record and reconcile the intent in the owner console. Only then is a receipt settled or declined.

If anything times out after submit, do not refresh, retry, or create a new idempotency key. Check KOHO first.

## Receiving money

KOHO's [third-party e-Transfer guide](https://help.koho.ca/en_us/what%27s-a-3rd-party-e-transfer-Bycs9jcUWe) describes the unique receiving email shown in the app. Copy only the address you deliberately want to make public into **Trust → Receiving**, with a memo such as `AGENT-{agent_id}-{intent_id}`.

An agent may share those public instructions. It cannot see the KOHO login identity unless you deliberately use the same address, and it cannot mark an arrival verified. Check the KOHO record yourself, then record the deposit in Cixa. The policy's reinvestment ratio and exposure ceilings decide how much, if any, becomes agent authority.

## When something looks wrong

End the Cixa payment session, stop all spending, and lock the card in KOHO. KOHO's [fraud guide](https://help.koho.ca/en_us/what-do-i-do-when-i-spot-a-fraudulent-transaction-on-my-account-rkHTqo98l) explains its card-lock and pending/settled distinction. Use only the official app for transaction alerts, verification, replacement, recovery, and disputes.

No repository test, screenshot, example, or demo uses a real card or makes a real transaction.

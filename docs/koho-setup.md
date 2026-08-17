# Manual KOHO Reference Setup

Last reviewed: 2026-08-16. KOHO features, fees, limits, product availability, and terms may change. This is an unaffiliated guide, not financial, legal, or compliance advice.

## Boundary

KOHO is used only as a manually operated provider for a user-owned Canadian prepaid virtual Mastercard. This project is not a KOHO API, partner, Mastercard or Interac product. Do not give the broker or an agent the KOHO password, verification code, recovery details, phone number used for authentication, government identity information, full PAN, expiry, or CVV.

## Owner Preparation

1. Create and verify the owner's KOHO account using the official KOHO app or website and the current [KOHO legal terms](https://www.koho.ca/legal/). The owner decides whether the account and card terms are acceptable.
2. Enable two-step verification, preferably the authenticator-app option described in KOHO's [official setup guide](https://help.koho.ca/en_us/set-up-two-step-verification-HkZqqo9IZx). Keep account identity and recovery information out of the agent environment.
3. Use a dedicated virtual card for this experiment and keep only a deliberately small amount exposed. KOHO describes virtual and physical cards in its [card comparison guide](https://help.koho.ca/en_us/whats-the-difference-between-a-virtual-card-and-a-physical-card-B1DFS2vEGv).
4. Configure applicable issuer-side card lock, transaction alert, and spending controls manually. Do not ask this project to unlock a card, approve a fraud alert, replace a card, or navigate the account.
5. Avoid activating borrowing, credit-building, overdraft, cash-advance, cover, pay-later, crypto, or other credit-like features for an agent test. Keep the project policy stricter than the issuer balance.

## Receiving Instructions

KOHO's [third-party e-Transfer guide](https://help.koho.ca/en_us/what%27s-a-3rd-party-e-transfer-Bycs9jcUWe) describes a unique email address shown in-app for receiving an Interac e-Transfer. The owner may copy that public address into the broker:

```bash
target/debug/cixa configure-receive --data-dir .local \
  --owner-token-file .local/owner.token \
  --method interac_e_transfer \
  --address 'owner-approved-public-address@example.invalid' \
  --memo-template 'AGENT-{agent_id}-{intent_id}'
```

Use a memo/reference format that does not disclose the login identity. The agent receives only the public address and memo template. It never receives a password, recovery information, authentication phone number, or card data. This release records notifications as unverified; only an owner-authenticated reconciliation makes incoming money spendable. It does not implement outgoing e-Transfers.

## Manual Purchase and Reconciliation

The owner must perform any real checkout in a separately reviewed, trusted context. Confirm merchant, origin, final amount, currency, recurring behavior, delivery destination, 3-D Secure, and issuer result in the official app. If the result is ambiguous, leave the intent `unknown` and reconcile it manually. Never refresh or resubmit.

After a risky run, lock the card in the official app. KOHO's [fraud guidance](https://help.koho.ca/en_us/what-do-i-do-when-i-spot-a-fraudulent-transaction-on-my-account-rkHTqo98l) describes locking and its pending/settled distinction. If a transaction alert appears, use only the in-app flow described by KOHO's [transaction-alert guidance](https://help.koho.ca/en_us/transaction-alerts%3A-what-you-need-to-know-r1Raco5Ll).

## No Credentials in Development

The local simulator, canary tests, and hostile merchant laboratory use synthetic values only. No development step asks for or stores real KOHO credentials, and no private KOHO endpoint is scraped, intercepted, reverse engineered, or automated.


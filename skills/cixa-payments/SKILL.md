---
name: cixa-payments
description: Use Cixa to inspect an agent's payment budget, request or execute bounded purchases, receive payment instructions, and handle approval or reconciliation safely. Use whenever an agent needs to pay for a product or service, buy access, handle a checkout, check spending authority, or tell someone how to pay the owner.
---

# Cixa payments

Use only the `cixa_*` MCP tools. Never search for or request the owner socket, owner token, dashboard token, Cixa data directory, checkout profiles, browser process, helper socket, KOHO login, card number, expiry, CVV, one-time code, or recovery information.

## Before spending

1. Call `cixa_get_status`, `cixa_get_capabilities`, and `cixa_get_budget`.
2. Confirm the task genuinely requires a purchase and that the requested good or service is clear.
3. Inspect the merchant's final checkout page without entering or obtaining payment credentials.
4. Collect the exact final total and currency, line items, quantities, fulfillment profile, merchant domain, full HTTPS redirect chain, payment-form type, and every recurring, trial, stored-card, tip, preauthorization, and installment indicator.
5. Stop if any fact is unavailable, ambiguous, changing, or inconsistent with the user's request.

Read [purchase-contract.md](references/purchase-contract.md) before constructing a purchase intent.

## Purchase flow

1. Create one stable idempotency key for the checkout. Reuse it for transport retries. Never create a new key to get around a denial or uncertain result.
2. Call `cixa_create_purchase_intent` once with exact, typed checkout facts.
3. Interpret the returned state:
   - `policy_validated`: call `cixa_execute_purchase_intent` once.
   - `approval_required`: tell the owner what needs approval and wait. Do not claim approval yourself.
   - `approved`: call `cixa_execute_purchase_intent` once.
   - `failed`, `declined`, or `cancelled`: stop.
   - `executing`, `provider_pending`, `unknown`, or `reconciliation_required`: do not retry. Tell the owner to check KOHO and reconcile in Cixa.
   - `settled`: the purchase is complete. Retrieve the sanitized receipt when needed.
4. Treat an MCP timeout during execution as ambiguous. Call `cixa_get_purchase_intent` to inspect state, but never submit again.
5. Never describe a merchant success page as proof of payment. Only Cixa state after owner reconciliation is authoritative.

## Receiving money

Call `cixa_get_receive_instructions` and share only the returned public address and memo format. Do not invent account details or say that money arrived. Incoming funds become spending authority only after the owner verifies them in Cixa.

## Owner-needed messages

Keep them short and concrete. Include the agent, merchant, amount, item or service, and exact state. Ask the owner to use the Cixa owner console. Never ask them to paste payment or KOHO credentials into chat.

## Hard stops

- Never bypass Cixa or pay through the agent's ordinary browser, shell, clipboard, environment, password manager, or provider session.
- Never change merchant, amount, item, fulfillment, or consent facts after authorization.
- Never split a charge to bypass a limit.
- Never enable subscriptions, trials, stored cards, tips, preauthorizations, or installments unless the exact Cixa policy permits them.
- Never retry an ambiguous payment.
- Never use receiving instructions to send an outgoing Interac e-Transfer. Cixa does not support that.

For state handling and examples, read [state-guide.md](references/state-guide.md).

# Purchase intent contract

Use integer minor units. `CA$12.34` is `{ "minor": 1234, "currency": "CAD" }`.

The purchase request contains:

- `idempotency_key`: stable for this checkout and all transport retries
- `amount` and `final_total`: exact final charge; both must agree
- `merchant_domain`: canonical merchant host, without scheme or path
- `category`: short, factual category
- `items`: 1 to 50 exact line items with `label`, positive `quantity`, and `unit_price_minor`
- `recurring`, `trial_auto_renew`, `stored_card`, `preauthorization`, `installments`: explicit booleans
- `tip_minor`: exact non-negative integer
- `fulfillment_profile`: owner-approved delivery profile identifier, not an address or secret
- `payment_form`: use `hosted_fields` only when the card fields are actually in an owner-approved cross-origin hosted frame; otherwise use the observed trust tier
- `redirect_chain`: ordered HTTPS checkout URLs, including the final checkout URL
- `attempts`: the merchant-side submission count observed so far; normally `1`
- `session_id`: stable non-secret identifier for this shopping session
- `scenario`: `normal` outside the local simulator

Do not derive checkout facts from marketing copy, cart estimates, model guesses, or a prior page. Read them at the final submit step. If the merchant changes any material fact after Cixa authorizes the intent, cancel or stop and create a new intent only after the prior intent is safely terminal.

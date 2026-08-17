# Cixa state guide

| State | Agent action |
| --- | --- |
| `draft`, `proposed` | Wait for Cixa to finish validation. |
| `policy_validated` | Execute once. |
| `approval_required` | Ask the owner to decide in Cixa, then re-read the intent. |
| `approved` | Execute once. |
| `funds_reserved`, `executing`, `provider_pending` | Wait and re-read. Never resubmit. |
| `unknown`, `reconciliation_required` | Stop. Owner checks KOHO and reconciles in Cixa. |
| `settled` | Complete. A sanitized receipt may be available. |
| `declined`, `failed`, `cancelled` | Stop. Do not work around the result. |
| `refunded` | Payment was reversed according to the reconciled Cixa record. |

## Example owner message

> Research Runner needs owner approval for CA$12.00 at icon-studio.example for the Interface icon set. Cixa has the exact checkout facts and is waiting in `approval_required`. Please decide in the Cixa owner console. Do not send card details here.

## Example ambiguous result

> Cixa submitted the CA$32.00 checkout once, but the outcome is `unknown`. I will not retry it. Please check the KOHO transaction list and reconcile this intent in the Cixa owner console.

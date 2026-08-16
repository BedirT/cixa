# ADR 0003: Tiered Checkout Trust and Safe Denial

## Status

Accepted.

## Decision

Recognize hosted fields first, permit explicit owner-approved merchant adapters, and treat unknown merchant-controlled payment forms as approval-required or denied in bounded-autonomous mode. Expose a narrow `CheckoutExecutor` contract but do not ship a universal browser driver.

## Rationale

An agent-controlled browser and a hostile merchant page cannot be trusted with card fields. A superficially universal automation layer would create a larger secret-exfiltration surface than the reference project can prove safe.

## Consequences

The project does not support every merchant. Future Playwright or equivalent work must demonstrate capture suppression, agent-control revocation, origin and total validation, ephemeral profiles, and exactly-once submission before being enabled.


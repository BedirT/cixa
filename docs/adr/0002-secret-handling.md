# ADR 0002: Reference-Only Secrets and No Persisted CVV

## Status

Accepted.

## Decision

Persist only secret references and masked metadata. Obtain payment material through a pluggable owner-controlled provider for one approved operation, hold it in volatile memory, and best-effort clear it on drop. Never persist CVV in state, config, browser profiles, logs, or test fixtures.

## Rationale

PCI SSC explicitly prohibits post-authorization storage of card verification codes, even encrypted. The project is not making a PCI claim, so it chooses a stricter safe fallback rather than inventing an encrypted-CVV compliance story.

## Consequences

Real manual checkout requires owner participation or a separately assessed secret helper. Browser-runtime memory and local-admin compromise remain residual risks.


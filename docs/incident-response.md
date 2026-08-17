# Incident Response

## Immediate Containment

1. Use the owner dashboard or CLI to turn on the emergency stop.
2. Stop the daemon and MCP process. Do not restart an unknown transaction.
3. Revoke the affected agent capability and preserve the agent ID, policy version, and intent IDs.
4. In the official issuer app, lock both relevant cards and follow the issuer's fraud procedure. For KOHO, use the official app, not an email or agent instruction.

## Credential Exposure

If PAN, CVV, owner token, agent token, or a login credential may have been exposed:

- treat it as compromised;
- do not paste it into an issue or chat;
- rotate or replace it through the owner/issuer interface;
- destroy temporary browser profiles, traces, screenshots, and crash artifacts;
- search generated artifacts with `./scripts/secret-canary-scan.py build` and inspect sanitized logs;
- record which boundaries were crossed without copying the secret itself.

## Ambiguous Payment

An `unknown` or `provider_pending` intent is quarantined. The owner checks the provider's official transaction view, captures only a sanitized reference and status, then runs:

```bash
target/debug/cixa reconcile --data-dir .local \
  --owner-token-file .local/owner.token --intent-id INTENT_ID --outcome settled
```

Use `declined` only when the provider confirms no charge. Use `refunded` only after a settled charge and a real refund is confirmed. Never execute the same intent again.

## Evidence and Recovery

Export the sanitized owner audit log, verify its HMAC chain, hash the exact binary and lockfiles, and record the current policy version. Reconcile every reservation before resuming. Emergency stop invalidates every broker-issued spending session, so the owner must explicitly re-arm each recovered agent after clearing the stop; old intents remain bound to the invalidated session and cannot resume. Reduce the exposed issuer balance, review merchant allowlists and fulfillment profiles, rotate tokens, and rerun the full canonical verification before a later dry run.

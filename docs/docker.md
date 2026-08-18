# Docker Deployment

Docker Compose is Cixa's primary deployment. It packages the broker, owner console, checkout browser, MCP bridge, fixed service identities, and two-volume trust boundary without placing a card or token value in Compose configuration.

## Start Cixa

Requirements:

- Docker Desktop, or Docker Engine with the Compose v2 plugin;
- enough disk for the Rust and Node build cache plus Chromium;
- an available loopback port, `8765` by default.

```bash
./scripts/cixa-docker up
./scripts/cixa-docker dashboard-token
```

Set `CIXA_DASHBOARD_PORT` before `up` to choose another loopback port. Cixa never binds the dashboard to a non-loopback address through the supplied Compose file.

The first build produces two targets:

- `cixa-owner:local` contains the Rust broker, owner console, checkout adapter, Node.js, Python, and a Playwright-managed Chromium;
- `cixa-agent:local` contains only the Node.js MCP runtime and agent SDK path.

Both use read-only root filesystems at runtime. Temporary space is a bounded, non-executable `tmpfs`. All Linux capabilities are dropped during normal operation, `no-new-privileges` is enabled, process counts are bounded, and only the broker receives a larger shared-memory allocation for Chromium.

## Create And Connect An Agent

Create an agent from the owner console. The capability value is written directly to the agent volume and never appears in the page or command output. Record the filename you chose.

```bash
./scripts/install-agent-skill --target all
./scripts/cixa-docker agent-config research-runner.token
```

The generated MCP command starts a disposable `cixa-mcp` container. It runs under UID `10001`, joins only IPC group `12000`, has no network namespace connectivity, and mounts `cixa-agent-ipc` read-only. It cannot inspect Docker, the host filesystem, or `cixa-owner-data`.

If the agent itself already runs in Docker, use the `agent` target or reproduce its invariant:

```yaml
user: "10001:10001"
group_add:
  - "12000"
read_only: true
network_mode: none # for the MCP sidecar, not necessarily the shopping agent
volumes:
  - cixa-agent-ipc:/run/cixa-agent:ro
```

Never mount `/var/run/docker.sock`, `cixa-owner-data`, an owner socket, or a host browser profile into an agent container.

## Volumes And Backups

`./scripts/cixa-docker down` preserves both named volumes. `cixa-owner-data` is the durable financial record and must be treated as one authenticated unit. Do not copy individual JSON fields or delete selected intent records.

For an owner-controlled backup:

1. End the payment session and stop Cixa.
2. Copy the complete `cixa-owner-data` volume into encrypted owner-controlled storage.
3. Keep the image version, repository commit, and generated SBOM with the backup.
4. Preserve `cixa-agent-ipc` only if you intend to retain existing capability files; otherwise revoke and reissue capabilities after restore.

The repository intentionally has no one-line volume-destruction wrapper. If the owner deliberately removes the volumes with Docker tooling, the ledger, reconciliation history, credentials, profiles, and capabilities are irrecoverable unless backed up.

## Updates

Review the release and dependency changes, then run:

```bash
./scripts/cixa-docker down
./scripts/cixa-docker up
```

Compose rebuilds the images and reuses the owner volumes. State format compatibility remains a release responsibility. Do not point an older image at a newer data directory unless the release notes explicitly allow it.

## Health And Logs

```bash
./scripts/cixa-docker status
./scripts/cixa-docker logs
```

The broker health check performs an authenticated owner-socket audit read. The console health check loads only the static loopback page. Health checks never expose credentials or perform a transaction.

## Verification

```bash
./scripts/verify-container
```

The container gate uses a unique Compose project and random loopback port. It builds both targets, initializes fresh volumes, waits for authenticated broker and console health, checks the UI, creates a synthetic short-lived agent, and calls agent-safe tools through the network-disabled MCP container. Cleanup removes only those isolated verification volumes.

No Docker verification fixture configures a real provider, opens a card session, or contacts a merchant.

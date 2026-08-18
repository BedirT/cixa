# syntax=docker/dockerfile:1.7

ARG RUST_VERSION=1.89
ARG NODE_VERSION=24.15.0

FROM rust:${RUST_VERSION}-bookworm AS rust-builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY apps/daemon/Cargo.toml apps/daemon/Cargo.toml
COPY crates/domain/Cargo.toml crates/domain/Cargo.toml
COPY apps/daemon/src apps/daemon/src
COPY crates/domain/src crates/domain/src
RUN cargo build --locked --release --package cixa

FROM node:${NODE_VERSION}-bookworm-slim AS node-builder
WORKDIR /src
COPY package.json package-lock.json ./
COPY packages packages
RUN npm ci --ignore-scripts && npm run build

FROM node:${NODE_VERSION}-bookworm-slim AS runtime-base
ARG CIXA_OWNER_UID=10000
ARG CIXA_OWNER_GID=10000
ARG CIXA_AGENT_UID=10001
ARG CIXA_AGENT_GID=10001
ARG CIXA_IPC_GID=12000

RUN groupadd --gid "${CIXA_OWNER_GID}" cixa-owner \
    && groupadd --gid "${CIXA_AGENT_GID}" cixa-agent \
    && groupadd --gid "${CIXA_IPC_GID}" cixa-agent-ipc \
    && useradd --uid "${CIXA_OWNER_UID}" --gid "${CIXA_OWNER_GID}" --groups "${CIXA_IPC_GID}" --create-home --shell /usr/sbin/nologin cixa-owner \
    && useradd --uid "${CIXA_AGENT_UID}" --gid "${CIXA_AGENT_GID}" --groups "${CIXA_IPC_GID}" --create-home --shell /usr/sbin/nologin cixa-agent

WORKDIR /opt/cixa
COPY --from=node-builder /src/node_modules ./node_modules
COPY --from=node-builder /src/packages ./packages

ENV CIXA_AGENT_SOCKET=/run/cixa-agent/cixa.sock \
    CIXA_AGENT_TOKEN_FILE=/run/cixa-agent/tokens/default.token \
    CIXA_DATA_DIR=/var/lib/cixa \
    CIXA_IPC_GID=12000 \
    CIXA_OWNER_GID=10000 \
    CIXA_OWNER_UID=10000 \
    HOME=/tmp/cixa-home \
    NODE_ENV=production

ENTRYPOINT ["/usr/local/bin/cixa-container"]

FROM runtime-base AS agent
COPY scripts/container-entrypoint /usr/local/bin/cixa-container
RUN chmod 0755 /usr/local/bin/cixa-container
USER 10001:10001
CMD ["mcp"]

FROM runtime-base AS owner
USER root
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates python3 \
    && PLAYWRIGHT_BROWSERS_PATH=/opt/cixa/browsers node /opt/cixa/node_modules/playwright-core/cli.js install --with-deps chromium \
    && mkdir -p /opt/cixa/browser \
    && browser_path="$(PLAYWRIGHT_BROWSERS_PATH=/opt/cixa/browsers node --input-type=module -e "import { chromium } from 'playwright-core'; process.stdout.write(chromium.executablePath())")" \
    && ln -s "${browser_path}" /opt/cixa/browser/chrome \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rust-builder /src/target/release/cixa /usr/local/bin/cixa
COPY apps/owner-dashboard /opt/cixa/apps/owner-dashboard
COPY scripts/container-entrypoint /usr/local/bin/cixa-container
RUN chmod 0755 /usr/local/bin/cixa /usr/local/bin/cixa-container

USER 10000:10000
CMD ["broker"]

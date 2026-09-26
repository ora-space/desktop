# Container images for the Ora Node (one per sandbox) and the Controller. One build stage compiles
# every executable both images need, so building both targets compiles the workspace once:
#
#   docker build -f docker/Dockerfile --target node -t ora-node:local .
#   docker build -f docker/Dockerfile --target controller -t ora-controller:local .
#
# See docs/deployment/container-images.md for the runtime contract of each image.

# Same compiler as rust-toolchain.toml. RUSTUP_TOOLCHAIN selects the image's toolchain directly, so
# rustup does not download the rustfmt/clippy components the file lists for the lint tasks.
FROM rust:1.95.0-bookworm AS build
ENV RUSTUP_TOOLCHAIN=1.95.0
WORKDIR /src
COPY . .
# Cache mounts keep the registry and incremental target between builds; the binaries are copied out
# because a cache mount is not part of the resulting layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release \
      -p ora-node -p ora-process-host -p ora-process-guardian -p ora-controller && \
    mkdir -p /out && \
    cp target/release/ora-node target/release/ora-process-host \
       target/release/ora-process-guardian target/release/ora-controller /out/

FROM debian:bookworm-slim AS runtime
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates tzdata && \
    rm -rf /var/lib/apt/lists/*

# Sandbox image started by the Sandbox Server, one container per Workspace. The Sandbox Server passes
# the Node service configuration in ORA_NODE_CONFIG and mounts the Workspace volume on /var/lib/ora;
# the entrypoint runs the process host and the Node as the unprivileged `node` user.
FROM runtime AS node
RUN apt-get update && \
    apt-get install -y --no-install-recommends git jq tini && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --uid 1000 --create-home --home-dir /home/node --shell /usr/sbin/nologin node && \
    install -d -o node -g node -m 0700 /var/lib/ora /run/ora && \
    install -d -m 0755 /etc/ora /opt/ora/bin
# Root-owned and not group/other-writable: the host refuses a guardian under a writable ancestor and
# the Node refuses a writable clone Git configuration in release builds. The parent directories exist
# before these COPYs because `--chmod` would otherwise also apply to the directories it creates.
COPY --from=build --chmod=0755 /out/ora-node /out/ora-process-host /out/ora-process-guardian /opt/ora/bin/
COPY --chmod=0644 docker/clone.gitconfig /etc/ora/clone.gitconfig
COPY --chmod=0755 docker/node-entrypoint.sh /opt/ora/bin/node-entrypoint.sh
VOLUME /var/lib/ora
# tini reaps processes reparented to PID 1 (guardians and their descendants) and forwards the stop
# signal to the entrypoint, which orders the shutdown.
ENTRYPOINT ["/usr/bin/tini", "--", "/opt/ora/bin/node-entrypoint.sh"]

# Controller in cloud persistence mode; the deployment mounts its JSON configuration and passes the
# path with --config. It serves no listener in that mode and reaches Nodes only through the router.
FROM runtime AS controller
RUN useradd --uid 10001 --create-home --home-dir /home/controller --shell /usr/sbin/nologin controller && \
    install -d -o controller -g controller -m 0700 /var/lib/ora-controller
COPY --from=build --chmod=0755 /out/ora-controller /opt/ora/bin/
USER controller
ENTRYPOINT ["/opt/ora/bin/ora-controller"]

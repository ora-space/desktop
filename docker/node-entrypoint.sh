#!/bin/sh
# Sandbox Node entrypoint. The Sandbox Server passes the complete Node service configuration as JSON
# in ORA_NODE_CONFIG and mounts the Workspace volume that holds every state path it names.
#
# The root management process owns protected configuration and the Node's journals.
# Git workloads run as UID/GID 1000, an identity the Node takes between fork and exec, and so do
# Agent plugins and the processes they start when the configuration names an agent workload
# directory. The Node runs every Git command itself and starts its own `ora-reaper`, so it is the
# only process this entrypoint supervises.
set -eu

node_user=node
# Mount point of the Workspace volume (the Sandbox Server's -node-data-dir); every state path in the
# configuration lives below it.
data_root=/var/lib/ora
config_file=/run/ora/node.json

prepare() {
  if [ -z "${ORA_NODE_CONFIG:-}" ]; then
    echo "node-entrypoint: ORA_NODE_CONFIG is empty; this image is started by the Sandbox Server" >&2
    exit 64
  fi
  # Only the volume mount point is prepared. Existing journals retain their ownership and
  # services refuse unsafe legacy layouts instead of recursively rewriting user data.
  chown root:root "$data_root"
  chmod 0711 "$data_root"
  # The Node requires the clone root to exist before startup and never creates it.
  repository_root=$(printf '%s' "$ORA_NODE_CONFIG" | jq -er '.clone.repository_root')
  if [ ! -d "$repository_root" ]; then
    install -d -o root -g root -m 0755 "$repository_root"
  fi
  # Agents running as the workload user get per-session directories under this root-owned
  # directory; the Node validates it and never creates it. 0711 lets the workload user reach its
  # own session directory by name without listing or replacing the others.
  workload_directory=$(printf '%s' "$ORA_NODE_CONFIG" | jq -r '.agent.workload_directory // empty')
  if [ -n "$workload_directory" ]; then
    install -d -o root -g root -m 0711 "$workload_directory"
  fi
  umask 077
  printf '%s' "$ORA_NODE_CONFIG" | jq -e . >"$config_file"
  for name in node-cert node-key ca controller-cert; do
    case "$name" in
      node-cert) value=${ORA_NODE_CERT:?} ;;
      node-key) value=${ORA_NODE_KEY:?} ;;
      ca) value=${ORA_NODE_CA:?} ;;
      controller-cert) value=${ORA_CONTROLLER_CERT:?} ;;
    esac
    printf '%s' "$value" | base64 -d >"/run/ora/$name.pem"
    chmod 0600 "/run/ora/$name.pem"
  done
  unset ORA_NODE_CERT ORA_NODE_KEY ORA_NODE_CA ORA_CONTROLLER_CERT value
  if jq -e '.agent.model_proxy != null' "$config_file" >/dev/null; then
    for name in model-client model-client-key model-ca; do
      case "$name" in
        model-client) value=${ORA_MODEL_CLIENT_CERT:?} ;;
        model-client-key) value=${ORA_MODEL_CLIENT_KEY:?} ;;
        model-ca) value=${ORA_MODEL_CA:?} ;;
      esac
      printf '%s' "$value" | base64 -d >"/run/ora/$name.pem"
      chmod 0600 "/run/ora/$name.pem"
    done
  fi
  unset ORA_MODEL_CLIENT_CERT ORA_MODEL_CLIENT_KEY ORA_MODEL_CA value
  # The git-access identity is separate from WSS and model access; only the root Node presents it
  # to the Git Server, so it stays in the private management tmpfs like the other identities.
  if jq -e '.git_access != null' "$config_file" >/dev/null; then
    for name in git-client git-client-key git-ca; do
      case "$name" in
        git-client) value=${ORA_GIT_CLIENT_CERT:?} ;;
        git-client-key) value=${ORA_GIT_CLIENT_KEY:?} ;;
        git-ca) value=${ORA_GIT_CA:?} ;;
      esac
      printf '%s' "$value" | base64 -d >"/run/ora/$name.pem"
      chmod 0600 "/run/ora/$name.pem"
    done
  fi
  unset ORA_GIT_CLIENT_CERT ORA_GIT_CLIENT_KEY ORA_GIT_CA value
  unset ORA_NODE_CONFIG
  supervise
}

supervise() {
  node_pid=
  stopping=0
  trap 'stopping=1; if [ -n "$node_pid" ]; then kill -TERM "$node_pid" 2>/dev/null || true; fi' TERM INT
  /opt/ora/bin/ora-node "$config_file" &
  node_pid=$!
  if [ "$stopping" = 1 ]; then kill -TERM "$node_pid" 2>/dev/null || true; fi
  # `wait` returns early when a trapped signal arrives; keep waiting until the Node really exits.
  status=0
  while kill -0 "$node_pid" 2>/dev/null; do
    wait "$node_pid" && status=0 || status=$?
  done
  exit "$status"
}

case "${1:-}" in
  supervise) supervise ;;
  "") prepare ;;
  *)
    echo "usage: node-entrypoint.sh" >&2
    exit 64
    ;;
esac

#!/bin/sh
# Sandbox Node entrypoint. The Sandbox Server passes the complete Node service configuration as JSON
# in ORA_NODE_CONFIG and mounts the Workspace volume that holds every state path it names.
#
# The root management process owns protected configuration, Node and process-host journals.
# Git workloads run as UID/GID 1000 through the scoped process boundary, and so do Agent plugins
# and the processes they start when the configuration names an agent workload directory. Stop order matters:
# the Node stops first so it can close its managed scopes
# through a live host, and only then is the host stopped. Guardians outlive both by design; their
# journals stay on the volume and the next container recovers them.
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
  unset ORA_NODE_CONFIG
  supervise
}

# Succeeds once something accepts connections on the socket; a leftover socket file from an
# earlier container does not count.
accepting() {
  perl -MIO::Socket::UNIX -e 'exit(IO::Socket::UNIX->new(Peer => $ARGV[0]) ? 0 : 1)' "$1" 2>/dev/null
}

supervise() {
  host_dir=$(jq -er '.process.host_directory' "$config_file")
  # `create` refuses an existing directory and `recover` needs the original journal, so the choice
  # follows the volume; a failed recovery never falls back to creating a new host.
  if [ -e "$host_dir" ]; then host_mode=recover; else host_mode=create; fi
  /opt/ora/bin/ora-process-host "$host_mode" "$host_dir" /opt/ora/bin/ora-process-guardian &
  host_pid=$!
  node_pid=
  stopping=0
  trap 'stopping=1; if [ -n "$node_pid" ]; then kill -TERM "$node_pid" 2>/dev/null || true; fi' TERM INT

  attempts=0
  until accepting "$host_dir/host.sock"; do
    if [ "$stopping" = 1 ] || ! kill -0 "$host_pid" 2>/dev/null; then
      kill -TERM "$host_pid" 2>/dev/null || true
      wait "$host_pid" || true
      echo "node-entrypoint: process host did not become ready" >&2
      exit 1
    fi
    attempts=$((attempts + 1))
    if [ "$attempts" -gt 300 ]; then
      kill -TERM "$host_pid" 2>/dev/null || true
      wait "$host_pid" || true
      echo "node-entrypoint: process host not ready after 30s" >&2
      exit 1
    fi
    sleep 0.1
  done

  /opt/ora/bin/ora-node "$config_file" &
  node_pid=$!
  if [ "$stopping" = 1 ]; then kill -TERM "$node_pid" 2>/dev/null || true; fi
  # `wait` returns early when a trapped signal arrives; keep waiting until the Node really exits.
  status=0
  while kill -0 "$node_pid" 2>/dev/null; do
    wait "$node_pid" && status=0 || status=$?
  done
  kill -TERM "$host_pid" 2>/dev/null || true
  while kill -0 "$host_pid" 2>/dev/null; do
    wait "$host_pid" || true
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

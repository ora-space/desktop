#!/bin/sh
# Sandbox Node entrypoint. The Sandbox Server passes the complete Node service configuration as JSON
# in ORA_NODE_CONFIG and mounts the Workspace volume that holds every state path it names.
#
# As root it only prepares what the unprivileged services cannot: the private data root on a fresh
# volume and the configuration file. It then re-executes itself as `node` to supervise the process
# host and the Node. Stop order matters: the Node stops first so it can close its managed scopes
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
  # A volume created before its first mount can come up root-owned; only its root is adjusted,
  # never its contents, so a foreign volume is refused by the services instead of rewritten.
  chown "$node_user:$node_user" "$data_root"
  chmod 0700 "$data_root"
  # The Node requires the clone root to exist before startup and never creates it.
  repository_root=$(printf '%s' "$ORA_NODE_CONFIG" | jq -er '.clone.repository_root')
  if [ ! -d "$repository_root" ]; then
    install -d -o "$node_user" -g "$node_user" -m 0700 "$repository_root"
  fi
  umask 077
  printf '%s' "$ORA_NODE_CONFIG" | jq -e . >"$config_file"
  chown "$node_user:$node_user" "$config_file"
  unset ORA_NODE_CONFIG
  exec setpriv --reuid="$node_user" --regid="$node_user" --init-groups -- "$0" supervise
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

#!/usr/bin/env bash

# Reuse system/CI installations. Stage downloads beside the destination so a failed
# download never leaves a partial executable that the next run would mistake for buf.
set -eu
if command -v buf >/dev/null 2>&1; then
  buf --version
  exit 0
fi
command -v curl >/dev/null 2>&1 || { echo "curl is required to install buf" >&2; exit 1; }
buf_version=1.73.0
buf_bin_dir="$HOME/.local/bin"
mkdir -p "$buf_bin_dir"
buf_download=$(mktemp "$buf_bin_dir/.buf.XXXXXX")
trap 'rm -f "$buf_download"' EXIT
echo "Installing buf $buf_version to $buf_bin_dir/buf"
curl --fail --location --silent --show-error --retry 3 \
  "https://github.com/bufbuild/buf/releases/download/v${buf_version}/buf-$(uname -s)-$(uname -m)" \
  --output "$buf_download"
chmod 755 "$buf_download"
"$buf_download" --version
mv -f "$buf_download" "$buf_bin_dir/buf"

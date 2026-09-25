#!/usr/bin/env bash

# The contract must be exactly the pinned commit: lint and breaking checks belong to the Cloud
# repository, this side only proves it consumes what it declares.
set -e
status=$(git submodule status -- third_party/cloud)
case "$status" in
  " "*) ;;
  *) echo "third_party/cloud is not at its pinned commit (run task proto:init): $status"; exit 1;;
esac
if [ -n "$(git -C third_party/cloud status --porcelain -- proto)" ]; then
  echo "third_party/cloud/proto has local changes; the contract only comes from the pinned commit"; exit 1
fi

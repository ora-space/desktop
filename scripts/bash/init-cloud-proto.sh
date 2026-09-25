#!/usr/bin/env bash

# A fresh clone is partial so blobs outside the contract are never fetched; a submodule whose
# git directory already exists (deinitialized, or checked out in full by a generic tool) is
# reused and only moved to the pinned commit. Either way the working tree ends sparse.
set -e
path=third_party/cloud
if ! git rev-parse --resolve-git-dir "$path/.git" >/dev/null 2>&1 \
  && [ ! -d "$(git rev-parse --git-dir)/modules/$path" ]; then
  url=$(git config -f .gitmodules "submodule.$path.url")
  git clone --no-checkout --filter=blob:none --sparse "$url" "$path"
  git -C "$path" sparse-checkout set proto
  git submodule absorbgitdirs -- "$path"
fi
git submodule update --init -- "$path"
git -C "$path" sparse-checkout set proto

#!/usr/bin/env bash

# A fresh clone is partial so blobs outside the contract are never fetched; a submodule whose
# git directory already exists (deinitialized, or checked out in full by a generic tool) is
# reused and only moved to the pinned commit. Either way the working tree ends sparse. The pattern
# is non-cone because cone mode always checks out every root-level file, and only the contract
# under proto/ belongs in this working tree.
set -e
path=third_party/cloud
if ! git rev-parse --resolve-git-dir "$path/.git" >/dev/null 2>&1 \
  && [ ! -d "$(git rev-parse --git-dir)/modules/$path" ]; then
  url=$(git config -f .gitmodules "submodule.$path.url")
  git clone --no-checkout --filter=blob:none --sparse "$url" "$path"
  # --no-checkout leaves the index empty at the default branch head. When the pin is that same
  # commit, the update below has nothing to switch and would keep the empty index, reporting every
  # contract file as a staged deletion; rebuilding the index from trees fetches no blob.
  git -C "$path" reset -q
  git -C "$path" sparse-checkout set --no-cone '/proto/'
  git submodule absorbgitdirs -- "$path"
fi
git submodule update --init -- "$path"
git -C "$path" sparse-checkout set --no-cone '/proto/'

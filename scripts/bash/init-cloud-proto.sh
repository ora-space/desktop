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
  pin=$(git rev-parse ":$path")
  git clone --no-checkout --filter=blob:none --sparse "$url" "$path"
  # --no-checkout leaves both the index and the working tree empty. Neither a mixed reset nor the
  # update below writes a file that is unchanged between the default branch head and the pin, so
  # the contract would stay missing from disk. Narrow the pattern first, then detach at the pin and
  # reset hard: that fetches and writes only the pinned proto/ blobs.
  git -C "$path" sparse-checkout set --no-cone '/proto/'
  git -C "$path" update-ref --no-deref HEAD "$pin"
  git -C "$path" reset -q --hard
  git submodule absorbgitdirs -- "$path"
fi
git submodule update --init -- "$path"
git -C "$path" sparse-checkout set --no-cone '/proto/'

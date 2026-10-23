#!/usr/bin/env bash
# Bump the workspace version, commit, and tag. Pushing the tag triggers the
# Release workflow, which builds every platform and publishes a GitHub Release.
#   scripts/release.sh 0.2.0 [--push]
set -euo pipefail

version="${1:?usage: scripts/release.sh <version> [--push]}"
version="${version#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "not semver: $version" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "working tree not clean" >&2; exit 1; }

sed -i.bak -E "/^\[workspace\.package\]/,/^\[/ s/^version = \".*\"/version = \"${version}\"/" Cargo.toml && rm Cargo.toml.bak
cargo update --workspace --quiet
git commit -qam "Release v${version}"
git tag -a "v${version}" -m "GitChrono v${version}"
echo "Tagged v${version}"

if [ "${2:-}" = "--push" ]; then
  git push origin HEAD "v${version}"
else
  echo "Push with: git push origin HEAD v${version}"
fi

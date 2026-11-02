#!/usr/bin/env bash
# Sets the version of every crate: the workspace version and the versions of the crates' path
# dependencies on each other, then refreshes Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?usage: scripts/set-version.sh <version>}
current=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[ "$current" = "$version" ] && exit 0
# `\r?` because Windows checkouts have CRLF line endings.
perl -0pi -e "s/(\[workspace\.package\]\r?\nversion = )\"[^\"]*\"/\${1}\"$version\"/" Cargo.toml
for manifest in crates/*/Cargo.toml; do
  perl -pi -e "s/(path = \"\.\.\/fml-[a-z]+\", version = )\"[^\"]*\"/\${1}\"$version\"/" "$manifest"
done
cargo update --workspace --quiet
echo "version $current -> $version"

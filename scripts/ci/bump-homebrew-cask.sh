#!/bin/bash
# Renders packaging/homebrew/asset-hub.rb for a release and pushes it to the Homebrew tap.
#
#   HOMEBREW_TAP_TOKEN=<token> scripts/ci/bump-homebrew-cask.sh <version> <asset file>
#
# HOMEBREW_TAP_REMOTE replaces the tap URL (a local bare repo for a dry run); the token is then
# not needed. The token is never printed.
set -euo pipefail

cask="asset-hub"
tap_repo="ZeeMRx0/homebrew-tap"

version="${1:-}"
asset="${2:-}"
[[ -n "$version" && -n "$asset" ]] || { echo "usage: $0 <version> <asset file>" >&2; exit 2; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "bump-homebrew-cask: '$version' is not a stable version (x.y.z)" >&2; exit 2; }
[[ -f "$asset" ]] || { echo "bump-homebrew-cask: asset file not found: $asset" >&2; exit 2; }

root="$(cd "$(dirname "$0")/../.." && pwd)"
template="$root/packaging/homebrew/$cask.rb"
[[ -f "$template" ]] || { echo "bump-homebrew-cask: template not found: $template" >&2; exit 2; }

remote="${HOMEBREW_TAP_REMOTE:-}"
if [[ -z "$remote" ]]; then
  [[ -n "${HOMEBREW_TAP_TOKEN:-}" ]] || { echo "bump-homebrew-cask: HOMEBREW_TAP_TOKEN is empty. Add it as a repository secret (fine-grained token, Contents read/write on $tap_repo)." >&2; exit 1; }
  remote="https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/${tap_repo}.git"
fi

sha="$(shasum -a 256 "$asset" | cut -d' ' -f1)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Output is silenced so a failing clone or push cannot print the token-bearing URL.
git clone --quiet --depth 1 --branch main "$remote" "$work/tap" >/dev/null 2>&1 || { echo "bump-homebrew-cask: could not clone the tap (check the token and its access)" >&2; exit 1; }
cd "$work/tap"
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

mkdir -p Casks
sed -e "s/__VERSION__/$version/" -e "s/__SHA256__/$sha/" "$template" > "Casks/$cask.rb"

git add "Casks/$cask.rb"
if git diff --cached --quiet; then
  echo "bump-homebrew-cask: $cask $version is already in the tap; nothing to do"
  exit 0
fi
git commit --quiet -m "$cask $version"

for attempt in 1 2 3; do
  if git push --quiet origin HEAD:main >/dev/null 2>&1; then
    echo "bump-homebrew-cask: pushed $cask $version (sha256 $sha)"
    exit 0
  fi
  echo "bump-homebrew-cask: push rejected (attempt $attempt of 3), rebasing" >&2
  git pull --quiet --rebase origin main >/dev/null 2>&1 || { echo "bump-homebrew-cask: rebase failed" >&2; exit 1; }
done
echo "bump-homebrew-cask: push failed after 3 attempts" >&2
exit 1

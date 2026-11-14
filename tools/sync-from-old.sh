#!/usr/bin/env bash
# Bring an old repository's newer main into python/<repo>/, with its history:
#
#   tools/sync-from-old.sh <repo> [<url>]
#
# <repo> is one of ooxml-common, ooxml-edit, docx2svg, pptx2svg, docx-agent, pptx-agent,
# xlsx-agent; <url> defaults to https://github.com/uvrt/<repo>.git.  Until the switch, fixes
# keep landing in the seven old repositories; this fetches the repository's main and merges
# it with a merge commit (`git merge -X subtree=python/<repo>/`), so the old commits keep
# their hashes and `git log <merge>^2` shows them.  Nothing is rewritten.
#
# Run it on a branch of this repository (never main; the result goes in by pull request).
# The import removed each package's .github/workflows (the root workflows replace them) and
# docx-agent's tracked .mypy_cache; a newer main that changes those files would conflict
# (modified there, deleted here), so those paths are kept deleted.  Any other conflict
# stops the script with the merge in progress, for a person to resolve and commit (or
# `git merge --abort`).
set -euo pipefail

repo=${1:?usage: tools/sync-from-old.sh <repo> [<url>]}
url=${2:-https://github.com/uvrt/$repo.git}
prefix=python/$repo
case $repo in
  ooxml-common|ooxml-edit|docx2svg|pptx2svg|docx-agent|pptx-agent|xlsx-agent) ;;
  *) echo "unknown repo: $repo" >&2; exit 2 ;;
esac
cd "$(git rev-parse --show-toplevel)"
branch=$(git symbolic-ref --short HEAD)
if [ "$branch" = main ]; then echo "on main: run this on a branch" >&2; exit 2; fi
if ! git diff --quiet || ! git diff --cached --quiet; then echo "uncommitted changes" >&2; exit 2; fi

git fetch --no-tags "$url" main
sha=$(git rev-parse FETCH_HEAD)
short=$(git rev-parse --short FETCH_HEAD)
if git merge-base --is-ancestor "$sha" HEAD; then
  echo "$repo $short is already in $branch; nothing to do"
  exit 0
fi

message="Sync $repo from its old repository at $short

$repo's main as of $sha, merged into $prefix/ with -X subtree (tools/sync-from-old.sh)."

if git merge --no-ff --no-commit -X "subtree=$prefix/" "$sha"; then
  :
else
  # Only the paths the import removed may conflict; anything else is for a person.
  others=$(git diff --name-only --diff-filter=U | grep -vE "^$prefix/(\.github/workflows/|\.mypy_cache/)" || true)
  if [ -n "$others" ]; then
    echo "conflicts to resolve by hand (the merge is in progress):" >&2
    echo "$others" >&2
    exit 1
  fi
fi
# Keep the removed paths removed, whether or not they conflicted.
for gone in "$prefix/.github/workflows" "$prefix/.mypy_cache"; do
  if [ -n "$(git ls-files -- "$gone")" ]; then git rm -r -q --cached -- "$gone"; rm -rf -- "$gone"; fi
done
# The subtree merge must not have put anything outside the prefix.
outside=$(git diff --cached --name-only HEAD | grep -v "^$prefix/" || true)
if [ -n "$outside" ]; then
  echo "the merge touches paths outside $prefix/ (the merge is in progress):" >&2
  echo "$outside" >&2
  exit 1
fi
git commit -q -m "$message"
echo "merged $repo $short into $branch as $(git rev-parse --short HEAD)"
git diff --stat HEAD^1 HEAD | tail -1

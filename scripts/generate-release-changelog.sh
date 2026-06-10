#!/usr/bin/env bash
set -euo pipefail

CURRENT_TAG="${1:-${GITHUB_REF_NAME:-}}"
if [[ -z "$CURRENT_TAG" ]]; then
  echo "Usage: $0 <tag>" >&2
  exit 1
fi

PREV_TAG="$(git tag -l 'v*' --sort=-v:refname | grep -Fxv "$CURRENT_TAG" | head -n 1 || true)"

if [[ -n "$PREV_TAG" ]]; then
  echo "### Changes since ${PREV_TAG}"
  echo ""
  git log "${PREV_TAG}..${CURRENT_TAG}" --pretty=format:'- %s' --no-merges --reverse
else
  echo "### Changes"
  echo ""
  git log "${CURRENT_TAG}" --pretty=format:'- %s' --no-merges --reverse
fi

echo ""
echo ""
echo "Desktop builds for Linux x64, macOS arm64/x64, and Windows x64."

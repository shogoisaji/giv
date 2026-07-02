#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: scripts/generate-homebrew-formula.sh OWNER TAG SHA256 [OUTPUT]

Example:
  scripts/generate-homebrew-formula.sh shogoisaji v0.1.0 <sha256> /path/to/homebrew-tap/Formula/giv.rb
EOF
}

if [[ $# -lt 3 || $# -gt 4 ]]; then
  usage
  exit 2
fi

owner="$1"
tag="$2"
sha256="$3"
output="${4:-Formula/giv.rb}"

case "$owner" in
  ""|*[!A-Za-z0-9_.-]*)
    echo "OWNER must be a GitHub owner or organization name" >&2
    exit 2
    ;;
esac

case "$tag" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *)
    echo "TAG must look like v0.1.0" >&2
    exit 2
    ;;
esac

if [[ ! "$sha256" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "SHA256 must be 64 hexadecimal characters" >&2
  exit 2
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
template="$repo_root/Formula/giv.rb.template"

if [[ ! -f "$template" ]]; then
  echo "missing template: $template" >&2
  exit 1
fi

escape_sed_replacement() {
  printf '%s' "$1" | sed 's/[\/&]/\\&/g'
}

owner_escaped="$(escape_sed_replacement "$owner")"
tag_escaped="$(escape_sed_replacement "$tag")"
sha_escaped="$(escape_sed_replacement "$sha256")"

mkdir -p "$(dirname -- "$output")"
sed \
  -e "s/OWNER/$owner_escaped/g" \
  -e "s/v0\.1\.0/$tag_escaped/g" \
  -e "s/REPLACE_WITH_RELEASE_TARBALL_SHA256/$sha_escaped/g" \
  "$template" > "$output"

if rg -q 'OWNER|REPLACE_WITH_RELEASE_TARBALL_SHA256' "$output"; then
  echo "generated formula still contains a placeholder: $output" >&2
  exit 1
fi

ruby -c "$output" >/dev/null
echo "wrote $output"

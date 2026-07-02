#!/usr/bin/env bash
set -euo pipefail

# Non-cargo release hygiene checks. The cargo build/test/clippy/fmt gates
# live in the CI and Release workflows directly; this script only verifies
# packaging-adjacent invariants (metadata, formula, YAML, shell syntax,
# git-attr / untracked-local-file policy).

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "==> package metadata"
cargo metadata --locked --format-version 1 --no-deps |
  ruby -rjson -e '
    pkg = JSON.parse(STDIN.read).fetch("packages").find { |p| p.fetch("name") == "giv" }
    abort("missing giv package metadata") unless pkg
    %w[license description readme repository homepage rust_version].each do |field|
      value = pkg[field]
      abort("Cargo.toml metadata field #{field} must be set") if value.nil? || value == ""
    end
    abort("giv is released through Homebrew; Cargo.toml publish must remain unset") unless pkg["publish"].nil?
  '

echo "==> shell syntax"
bash -n scripts/generate-homebrew-formula.sh
bash -n scripts/release-check.sh

echo "==> GitHub workflow and issue template YAML"
ruby -e "require 'yaml'; Dir['.github/**/*.yml'].sort.each { |f| YAML.load_file(f) }"

echo "==> ruby -c Formula/giv.rb.template"
ruby -c Formula/giv.rb.template >/dev/null

echo "==> formula generation smoke test"
tmp_formula="$(mktemp)"
scripts/generate-homebrew-formula.sh \
  example-owner \
  v0.1.0 \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$tmp_formula" >/dev/null
rm -f "$tmp_formula"

echo "==> git export-ignore attributes"
for path in AGENTS.md .DS_Store; do
  attr="$(git check-attr export-ignore -- "$path")"
  case "$attr" in
    *"export-ignore: set") ;;
    *)
      echo "expected $path to have export-ignore set, got: $attr" >&2
      exit 1
      ;;
  esac
done

echo "==> local-only files are not tracked"
for path in AGENTS.md .DS_Store; do
  if git ls-files --error-unmatch "$path" >/dev/null 2>&1; then
    echo "$path must not be tracked in the public repository" >&2
    exit 1
  fi
done

echo "release check passed"

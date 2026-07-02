# Homebrew release checklist

giv is released through a Homebrew tap, not crates.io.

## Before tagging

Run the local quality gates:

```sh
./scripts/release-check.sh
```

Confirm the GitHub source archive will not export local-only files:

```sh
git check-attr export-ignore -- AGENTS.md .DS_Store
```

Both files should report `export-ignore: set`.

## Tag and GitHub release

1. Update `version` in `Cargo.toml` if needed.
2. Commit the release.
3. Create and push a signed tag, for example `v0.1.0`.
4. The `Release` workflow creates a draft GitHub release with release assets
   and `SHA256SUMS`.
5. Review the draft release notes and publish the GitHub release.

## Homebrew tap update

Use the source archive SHA-256 from the release workflow summary or
`SHA256SUMS`. To compute it manually:

```sh
curl -L https://github.com/OWNER/giv/archive/refs/tags/v0.1.0.tar.gz | shasum -a 256
```

Generate the formula into the tap:

```sh
./scripts/generate-homebrew-formula.sh OWNER v0.1.0 SHA256 /path/to/homebrew-tap/Formula/giv.rb
```

Validate the formula from the tap:

```sh
brew install --build-from-source ./Formula/giv.rb
brew test giv
brew audit --strict --online giv
```

#!/bin/bash
#
# Release script for tauri-plugin-dev-invoke.
#
#   ./scripts/release.sh [major|minor|patch] [--dry-run]
#
# Nothing leaves this machine until both registries have accepted the release: the commit and
# tag are made locally, the packages are published, and only then is anything pushed. If a
# publish fails you are left with a local commit and tag to unwind, rather than a tag on
# GitHub pointing at a release that does not exist.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
DIM='\033[2m'
NC='\033[0m'

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CRATE_DIR="$ROOT_DIR/packages/tauri-plugin-dev-invoke"
API_DIR="$ROOT_DIR/packages/tauri-plugin-dev-invoke-api"
CHANGELOG="$ROOT_DIR/CHANGELOG.md"
RELEASE_BRANCH="main"

BUMP_TYPE="patch"
DRY_RUN=0

for arg in "$@"; do
  case "$arg" in
    major|minor|patch) BUMP_TYPE="$arg" ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help)
      awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' "$0"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown argument '$arg'. Usage: $0 [major|minor|patch] [--dry-run]${NC}" >&2
      exit 1
      ;;
  esac
done

# How far the release got, so the exit handler can say the right thing. macOS ships bash 3.2,
# which does not run ERR traps for failing subshells (and `set -E` does not help), so recovery
# advice hangs off EXIT instead.
RELEASE_STAGE="preflight"

on_exit() {
  local code=$?
  [[ $code -eq 0 ]] && return

  case "$RELEASE_STAGE" in
    committed)
      echo -e "\n${RED}Release failed before publishing.${NC} Nothing was published or pushed." >&2
      echo -e "Unwind the local commit and tag with:\n  git reset --hard HEAD~1\n  git tag -d $TAG" >&2
      ;;
    crate-published)
      echo -e "\n${RED}Release failed after crates.io accepted it.${NC} npm was not published, and nothing was pushed." >&2
      echo -e "${YELLOW}tauri-plugin-dev-invoke $NEW_VERSION is now permanent on crates.io${NC} — that version cannot be published again." >&2
      echo -e "Either finish by hand:\n  cd $API_DIR && npm publish\n  git push origin $RELEASE_BRANCH && git push origin $TAG" >&2
      echo -e "or yank it (cargo yank --version $NEW_VERSION) and release the next patch instead." >&2
      ;;
    published)
      echo -e "\n${RED}Both packages published, but the push failed.${NC}" >&2
      echo -e "Do not re-run this script — just push:\n  git push origin $RELEASE_BRANCH\n  git push origin $TAG" >&2
      ;;
  esac
}
trap on_exit EXIT

step()  { echo -e "\n${YELLOW}==> $*${NC}"; }
info()  { echo -e "    ${DIM}$*${NC}"; }
ok()    { echo -e "    ${GREEN}✓${NC} $*"; }
die()   { echo -e "\n${RED}✗ $*${NC}" >&2; exit 1; }

# BSD sed wants an explicit empty suffix for -i; GNU sed must not have one.
sed_inplace() {
  if sed --version >/dev/null 2>&1; then
    sed -i "$1" "$2"
  else
    sed -i '' "$1" "$2"
  fi
}

# Rewrites a version string and fails if the pattern did not actually match, so a drifted
# format can never publish the previous version by accident.
replace_or_die() {
  local expr="$1" file="$2" expect="$3" label="$4"
  sed_inplace "$expr" "$file"
  grep -q "$expect" "$file" || die "could not update the version in $label
The file's format has drifted from what this script expects. Fix it by hand, or update the
pattern here, then re-run."
  ok "$label"
}

# ---------------------------------------------------------------------------------------
# preflight
# ---------------------------------------------------------------------------------------

step "Preflight"
cd "$ROOT_DIR"

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$CURRENT_BRANCH" != "$RELEASE_BRANCH" ]]; then
  if [[ $DRY_RUN -eq 1 ]]; then
    info "on '$CURRENT_BRANCH', not '$RELEASE_BRANCH' (allowed for a dry run)"
  else
    die "releases are cut from '$RELEASE_BRANCH', but you are on '$CURRENT_BRANCH'."
  fi
else
  ok "on $RELEASE_BRANCH"
fi

# A dirty tree would be swept into the release commit, and `cargo publish` refuses it anyway.
[[ -z "$(git status --porcelain)" ]] || die "the working tree has uncommitted changes.
Commit or stash them first — otherwise they end up in the release commit.

$(git status --short)"
ok "working tree is clean"

# Checked here rather than alongside the edits, so an undocumented release is rejected before
# spending several minutes on a release build.
if [[ -f "$CHANGELOG" ]]; then
  grep -q '^## \[Unreleased\]' "$CHANGELOG" || die "CHANGELOG.md has no '## [Unreleased]' section.
Add one describing this release before publishing."

  UNRELEASED_BODY="$(awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{f=0} f' "$CHANGELOG" | tr -d '[:space:]')"
  [[ -n "$UNRELEASED_BODY" ]] || die "CHANGELOG.md's '## [Unreleased]' section is empty.
Describe what is in this release before publishing it."
  ok "changelog has unreleased notes"
fi

git fetch --quiet origin "$RELEASE_BRANCH" 2>/dev/null || info "could not reach origin, skipping the sync check"
if git rev-parse --verify --quiet "origin/$RELEASE_BRANCH" >/dev/null; then
  BEHIND="$(git rev-list --count "HEAD..origin/$RELEASE_BRANCH")"
  [[ "$BEHIND" -eq 0 ]] || die "origin/$RELEASE_BRANCH is $BEHIND commit(s) ahead. Pull first."
  ok "up to date with origin/$RELEASE_BRANCH"
fi

# ---------------------------------------------------------------------------------------
# versions
# ---------------------------------------------------------------------------------------

step "Version"

CURRENT_VERSION="$(grep -m1 '^version = "' "$CRATE_DIR/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
[[ -n "$CURRENT_VERSION" ]] || die "could not read the current version from $CRATE_DIR/Cargo.toml"

API_VERSION="$(grep -m1 '"version"' "$API_DIR/package.json" | sed 's/.*"version": "\(.*\)".*/\1/')"
[[ "$API_VERSION" == "$CURRENT_VERSION" ]] || die "the packages disagree on the current version.
  crate:  $CURRENT_VERSION
  npm:    $API_VERSION
They are released together, so bring them back in step before releasing."

IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"
case $BUMP_TYPE in
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  patch) PATCH=$((PATCH + 1)) ;;
esac
NEW_VERSION="$MAJOR.$MINOR.$PATCH"
TAG="v$NEW_VERSION"

! git rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null || die "tag $TAG already exists locally."
! git ls-remote --exit-code --tags origin "$TAG" >/dev/null 2>&1 || die "tag $TAG already exists on origin."

echo -e "    $CURRENT_VERSION -> ${GREEN}$NEW_VERSION${NC} ${DIM}($BUMP_TYPE)${NC}"

# ---------------------------------------------------------------------------------------
# credentials — checked now, so a missing token cannot strand a tagged release
# ---------------------------------------------------------------------------------------

step "Credentials"

if [[ -f "$HOME/.cargo/credentials.toml" || -f "$HOME/.cargo/credentials" || -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
  ok "crates.io token found"
else
  info "no crates.io token found; if you use a credential provider this is fine"
  info "otherwise: cargo login   (token from https://crates.io/settings/tokens)"
fi

if NPM_USER="$(npm whoami --prefix "$API_DIR" 2>/dev/null)"; then
  ok "npm authenticated as $NPM_USER"
else
  die "npm is not authenticated. Run: npm login"
fi

# ---------------------------------------------------------------------------------------
# quality gates — before the prompt, so you approve something that already passed
# ---------------------------------------------------------------------------------------

step "Checks"
cd "$CRATE_DIR"
cargo fmt --check && ok "rustfmt"
cargo clippy --all-targets --all-features -- -D warnings && ok "clippy"
cargo test --quiet && ok "tests"
cargo build --release --quiet && ok "release build"

cd "$API_DIR"
npm ci --silent && npm run --silent build && ok "api package builds"

# ---------------------------------------------------------------------------------------
# edits
# ---------------------------------------------------------------------------------------

step "Updating versions"
cd "$ROOT_DIR"

replace_or_die "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" \
  "$CRATE_DIR/Cargo.toml" "^version = \"$NEW_VERSION\"" "${CRATE_DIR#"$ROOT_DIR"/}/Cargo.toml"

replace_or_die "s/\"version\": \"$CURRENT_VERSION\"/\"version\": \"$NEW_VERSION\"/" \
  "$API_DIR/package.json" "\"version\": \"$NEW_VERSION\"" "${API_DIR#"$ROOT_DIR"/}/package.json"

for readme in "$ROOT_DIR/README.md" "$CRATE_DIR/README.md" "$API_DIR/README.md"; do
  [[ -f "$readme" ]] || continue
  replace_or_die "s/tauri-plugin-dev-invoke = \"[0-9]*\.[0-9]*\"/tauri-plugin-dev-invoke = \"$MAJOR.$MINOR\"/" \
    "$readme" "tauri-plugin-dev-invoke = \"$MAJOR.$MINOR\"" "${readme#"$ROOT_DIR"/}"
done

# Keep the lockfiles in step with the manifests, so the release commit is self-consistent.
(cd "$CRATE_DIR" && cargo check --quiet)
(cd "$API_DIR" && npm install --silent --package-lock-only)
ok "lockfiles"

step "Updating the changelog"

if [[ -f "$CHANGELOG" ]]; then
  TODAY="$(date +%Y-%m-%d)"
  awk -v version="$NEW_VERSION" -v date="$TODAY" '
    /^## \[Unreleased\]/ && !done {
      print "## [Unreleased]"
      print ""
      print "## [" version "] - " date
      done = 1
      next
    }
    { print }
  ' "$CHANGELOG" > "$CHANGELOG.tmp" && mv "$CHANGELOG.tmp" "$CHANGELOG"

  REPO_URL="$(git remote get-url origin | sed 's/\.git$//' | sed 's#git@github.com:#https://github.com/#')"
  if grep -q '^\[Unreleased\]:' "$CHANGELOG"; then
    sed_inplace "s#^\[Unreleased\]:.*#[Unreleased]: $REPO_URL/compare/$TAG...HEAD#" "$CHANGELOG"
    sed_inplace "s#^\[Unreleased\]: \(.*\)#[Unreleased]: \1\\
[$NEW_VERSION]: $REPO_URL/releases/tag/$TAG#" "$CHANGELOG"
  fi
  ok "released [Unreleased] as $NEW_VERSION ($TODAY)"
else
  info "no CHANGELOG.md, skipping"
fi

step "Review"
git --no-pager diff --stat
echo
git --no-pager diff -- "$CHANGELOG" | head -30

# ---------------------------------------------------------------------------------------
# publish
# ---------------------------------------------------------------------------------------

step "Publish dry run"
(cd "$CRATE_DIR" && cargo publish --dry-run --quiet --allow-dirty) && ok "cargo publish"
(cd "$API_DIR" && npm publish --dry-run >/dev/null 2>&1) && ok "npm publish"

if [[ $DRY_RUN -eq 1 ]]; then
  step "Dry run complete"
  info "reverting the version and changelog edits"
  git checkout -- .
  echo -e "\n${GREEN}Nothing was committed, tagged, published or pushed.${NC}"
  exit 0
fi

echo
read -p "$(echo -e "${YELLOW}Publish $TAG to crates.io and npm? This cannot be undone. (y/N) ${NC}")" confirm
if [[ ! $confirm =~ ^[Yy]$ ]]; then
  git checkout -- .
  echo "Aborted; edits reverted."
  exit 0
fi

step "Committing $TAG"
git add -- "$CRATE_DIR/Cargo.toml" "$CRATE_DIR/Cargo.lock" "$API_DIR/package.json" \
  "$API_DIR/package-lock.json" "$ROOT_DIR/README.md" "$CRATE_DIR/README.md" "$API_DIR/README.md"
[[ -f "$CHANGELOG" ]] && git add -- "$CHANGELOG"
git commit -qm "Release $TAG"
git tag -a "$TAG" -m "Release $TAG"
RELEASE_STAGE="committed"
ok "committed and tagged locally"

step "Publishing to crates.io"
(cd "$CRATE_DIR" && cargo publish)
RELEASE_STAGE="crate-published"
ok "tauri-plugin-dev-invoke@$NEW_VERSION"

step "Publishing to npm"
(cd "$API_DIR" && npm publish)
RELEASE_STAGE="published"
ok "tauri-plugin-dev-invoke-api@$NEW_VERSION"

step "Pushing"
git push -q origin "$RELEASE_BRANCH"
git push -q origin "$TAG"
RELEASE_STAGE="done"
ok "pushed $RELEASE_BRANCH and $TAG"

REPO_URL="$(git remote get-url origin | sed 's/\.git$//' | sed 's#git@github.com:#https://github.com/#')"
echo -e "\n${GREEN}✅ Released $TAG${NC}"
echo -e "   crates.io  https://crates.io/crates/tauri-plugin-dev-invoke/$NEW_VERSION"
echo -e "   npm        https://www.npmjs.com/package/tauri-plugin-dev-invoke-api/v/$NEW_VERSION"
echo -e "   ${DIM}Draft the GitHub release: $REPO_URL/releases/new?tag=$TAG${NC}"

#!/bin/bash
set -e

# Release script for tauri-plugin-dev-invoke
# Usage: ./scripts/release.sh [major|minor|patch]

BUMP_TYPE=${1:-patch}
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🚀 Starting release process...${NC}"

# Get current version from Cargo.toml
CURRENT_VERSION=$(grep -m1 'version = "' "$ROOT_DIR/packages/tauri-plugin-dev-invoke/Cargo.toml" | sed 's/.*version = "\([^"]*\)".*/\1/')
echo -e "Current version: ${YELLOW}$CURRENT_VERSION${NC}"

# Calculate new version
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"
case $BUMP_TYPE in
  major)
    MAJOR=$((MAJOR + 1))
    MINOR=0
    PATCH=0
    ;;
  minor)
    MINOR=$((MINOR + 1))
    PATCH=0
    ;;
  patch)
    PATCH=$((PATCH + 1))
    ;;
  *)
    echo -e "${RED}Error: Invalid bump type '$BUMP_TYPE'. Use 'major', 'minor', or 'patch'.${NC}"
    exit 1
    ;;
esac
NEW_VERSION="$MAJOR.$MINOR.$PATCH"
echo -e "New version: ${GREEN}$NEW_VERSION${NC}"

# Confirm with user
read -p "Proceed with release v$NEW_VERSION? (y/N) " confirm
if [[ ! $confirm =~ ^[Yy]$ ]]; then
  echo "Aborted."
  exit 0
fi

# Update Rust package version
echo -e "${YELLOW}Updating Cargo.toml...${NC}"
sed -i '' "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" "$ROOT_DIR/packages/tauri-plugin-dev-invoke/Cargo.toml"

# Update JS package version (if exists)
JS_PACKAGE="$ROOT_DIR/packages/tauri-plugin-dev-invoke-api/package.json"
if [[ -f "$JS_PACKAGE" ]]; then
  echo -e "${YELLOW}Updating package.json...${NC}"
  sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$NEW_VERSION\"/" "$JS_PACKAGE"
fi

# Update version in README files
echo -e "${YELLOW}Updating README files...${NC}"
ROOT_README="$ROOT_DIR/README.md"
PACKAGE_README="$ROOT_DIR/packages/tauri-plugin-dev-invoke/README.md"

for readme in "$ROOT_README" "$PACKAGE_README"; do
  if [[ -f "$readme" ]]; then
    # Update version in dependency examples like: tauri-plugin-dev-invoke = "0.2"
    sed -i '' "s/tauri-plugin-dev-invoke = \"[0-9]*\.[0-9]*\"/tauri-plugin-dev-invoke = \"$MAJOR.$MINOR\"/" "$readme"
  fi
done
echo -e "${YELLOW}Building and testing...${NC}"
cd "$ROOT_DIR/packages/tauri-plugin-dev-invoke"
cargo check
cargo build --release

# Commit changes
echo -e "${YELLOW}Committing changes...${NC}"
cd "$ROOT_DIR"
git add -A
git commit -m "Release v$NEW_VERSION"

# Create tag
echo -e "${YELLOW}Creating tag v$NEW_VERSION...${NC}"
git tag -a "v$NEW_VERSION" -m "Release v$NEW_VERSION"

# Push to remote
echo -e "${YELLOW}Pushing to remote...${NC}"
git push origin main
git push origin "v$NEW_VERSION"

# Publish to crates.io
echo -e "${YELLOW}Publishing to crates.io...${NC}"
cd "$ROOT_DIR/packages/tauri-plugin-dev-invoke"
cargo publish

# Publish to npm (if package.json exists)
if [[ -f "$JS_PACKAGE" ]]; then
  echo -e "${YELLOW}Publishing to npm...${NC}"
  cd "$ROOT_DIR/packages/tauri-plugin-dev-invoke-api"
  npm publish
fi

echo -e "${GREEN}✅ Release v$NEW_VERSION complete!${NC}"

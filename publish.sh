#!/bin/bash

# Deps:
# https://pypi.org/project/toml-cli/
# https://github.com/davidrjonas/semver-cli

set -e

# --- Utility Functions ---

function cleanup_and_exit {
    echo "Something went wrong. Reverting changes."
    # Revert to the last commit to clean up any changes made by the script
    # This is a bit aggressive but ensures a clean state
    git reset --hard HEAD
    exit 1
}

# Trap any exit signals (e.g., Ctrl+C) and run the cleanup function
trap cleanup_and_exit INT

# --- Pre-flight Checks ---

# Check for required dependencies
for cmd in toml semver-cli; do
    if ! command -v "$cmd" &> /dev/null; then
        echo "Error: Required command '$cmd' not found. Please install it."
        exit 1
    fi
done

# Check for uncommitted changes
if [[ -n $(git status --porcelain) ]]; then
  echo "Error: You have uncommitted changes. Please commit or stash them first."
  exit 1
fi

# --- Get and Validate Versions ---

CURRENT_VERSION=$(toml get package.version --toml-path router/Cargo.toml)
ROUTER_MACROS_VERSION=$(toml get package.version --toml-path router_macros/Cargo.toml)

if [ "$CURRENT_VERSION" != "$ROUTER_MACROS_VERSION" ]; then
    echo "Error: router_macros version ($ROUTER_MACROS_VERSION) does not match router version ($CURRENT_VERSION)."
    exit 1
fi

# --- Get User Input ---

read -n 1 -p "What type of publish is this?: (M)ajor, (m)inor, (p)atch: " PUBLISH_TYPE
echo

NEXT_VERSION=""
case "$PUBLISH_TYPE" in
    M) NEXT_VERSION=$(semver-cli inc major "$CURRENT_VERSION") ;;
    m) NEXT_VERSION=$(semver-cli inc minor "$CURRENT_VERSION") ;;
    p) NEXT_VERSION=$(semver-cli inc patch "$CURRENT_VERSION") ;;
    *) echo "Error: Unknown upgrade type '$PUBLISH_TYPE'."; exit 1 ;;
esac

echo "Will perform update from $CURRENT_VERSION to $NEXT_VERSION."

read -p "Ok to proceed? (y/N): " CONFIRM
if [ "$CONFIRM" != "y" ]; then
    echo "Cancelled by user. Exiting."
    exit 1
fi

# --- Update TOML files ---

echo "Updating Cargo.toml files..."
toml set dependencies.milrouter_macros.version \"$NEXT_VERSION\" --toml-path router/Cargo.toml
toml set package.version \"$NEXT_VERSION\" --toml-path router_macros/Cargo.toml
toml set package.version \"$NEXT_VERSION\" --toml-path router/Cargo.toml

# --- Dry Run Publish ---

echo "Performing dry run of cargo publish..."
if ! cargo +nightly publish -p milrouter -p milrouter_macros --dry-run; then
    echo "Cargo dry-run failed. Exiting without changes."
    exit 1
fi

# --- Commit and Push ---

echo "Committing version bump..."
git add .
git commit -am "Bump version $CURRENT_VERSION -> $NEXT_VERSION"

echo "Pushing changes..."
if ! git push; then
    echo "Git push failed. Please resolve manually."
    exit 1
fi

# --- Final Publish ---

echo "Publishing crates..."
cargo +nightly publish -p milrouter -p milrouter_macros

echo "✅ Publishing process complete. Version $NEXT_VERSION is now live!"
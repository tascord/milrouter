#!/bin/bash

# Deps:
# https://pypi.org/project/toml-cli/
# https://github.com/davidrjonas/semver-cli

set -e

CURRENT_VERSION=$(toml get package.version --toml-path router/Cargo.toml)

# Sanity check
if ["$CURRENT_VERSION" != "$(toml get package.version --toml-path router_macros/Cargo.toml)"]; then
    echo "Error: router_macros not on expected version '$CURRENT_VERSION'."
    exit 1
fi

# Check git is in a good state
if [[ -n $(git status --porcelain) ]]; then
  echo "Error: You have uncommitted changes. Please commit or stash them first."
  exit 1
fi

# Bump versions of packages
TYPE=read -n 1 -p "What type of publish is this?: (M)aj, (m)in (p)atch"
NEXT_VERSION=""

if [ TYPE = "M" ]; then
    NEXT_VERSION=$(semver-cli inc major $CURRENT_VERSION)
elif [ TYPE = "m" ]; then
    NEXT_VERSION=$(semver-cli inc minor $CURRENT_VERSION)
elif [ TYPE = "p" ]; then
    NEXT_VERSION=$(semver-cli inc patch $CURRENT_VERSION)
else
    echo "Unknown upgrade type: $TYPE"
    exit 1
fi

echo "Will perform update $CURRENT_VERSION --> $NEXT_VERSION"
if [ read -p "Ok? y/N:" != "y" ]; then
    echo "Error: Cancelled by user."
    exit 1
fi

toml set dependencies.milrouter_macros.version $NEXT_VERSION --toml-path router/Cargo.toml
toml set package.version $NEXT_VERSION --toml-path router_macros/Cargo.toml
toml set package.version $NEXT_VERSION --toml-path router/Cargo.toml

# Sanity check `cargo publish`
set +e
cargo +nightly publish -p milrouter -p milrouter_macros --dry-run
EXIT=$?
set -e

if [ $EXIT != 0 ]; then
    toml set dependencies.milrouter_macros.version $CURRENT_VERSION --toml-path router/Cargo.toml
    toml set package.version $CURRENT_VERSION --toml-path router_macros/Cargo.toml
    toml set package.version $CURRENT_VERSION --toml-path router/Cargo.toml
    echo "Cargo dry-run failed. Reverted package changes."
    exit 1
fi

echo "Comitting + Pushing version bump"
git add .
git commit -am "Bump version $CURRENT_VERSION -> $NEXT_VERSION"
git push

echo "Publishing"
cargo +nightly publish -p milrouter -p milrouter_macros
#!/bin/bash
# Fixed package names come from the Rust Tool enum. Never accept UI-supplied shell code.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
for package in "$@"; do
    case "$package" in
        gcc|g++|gfortran|make|cmake|pkg-config) ;;
        *) exit 64 ;;
    esac
done

# Preserve recovery errors (including a held dpkg lock) instead of continuing blindly.
dpkg --configure -a
apt-get -o DPkg::Lock::Timeout=120 -o APT::Update::Error-Mode=any update

# Compiler meta-packages often own no binaries. Find damaged installed dependencies too,
# so reinstalling gcc actually restores a missing gcc-<version> executable or C header.
# Package names come from apt's trusted dependency metadata, never from probe output.
dependencies=$(apt-cache depends --recurse --installed --important "$@")
repair=("$@")
while IFS= read -r package; do
    [[ "$package" =~ ^[a-z0-9][a-z0-9+.-]*(:[a-z0-9-]+)?$ ]] || continue
    if dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -qx 'install ok installed'; then
        verification=$(dpkg --verify "$package")
        # apt preserves local configuration files. Only payload damage calls for repair.
        if printf '%s\n' "$verification" | grep -Ev '^[[:space:]]*$| c ' | grep -q .; then
            repair+=("$package")
        fi
    fi
done <<< "$dependencies"

# Never remove another package to solve dependencies; retain local configuration.
apt-get -o DPkg::Lock::Timeout=120 -o Dpkg::Options::=--force-confold \
    install -y --no-remove --no-install-recommends --reinstall "${repair[@]}"

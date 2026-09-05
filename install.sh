#!/usr/bin/env bash
# Strata Universal Installer for macOS and Linux
# Usage: curl -fsSL https://raw.githubusercontent.com/phfarath/Strata/main/install.sh | bash

set -e

REPO="phfarath/Strata"
BOLD="\033[1m"
GREEN="\033[0;32m"
BLUE="\033[0;34m"
RED="\033[0;31m"
YELLOW="\033[0;33m"
RESET="\033[0m"

echo -e "${BLUE}${BOLD}"
echo "  ___ _             _        "
echo " / __| |_ _ _ __ _| |_ __ _ "
echo " \__ \  _| '_/ _\` |  _/ _\` |"
echo " |___/\__|_| \__,_|\__\__,_|"
echo -e "${RESET}"
echo -e "  Local-First Persistent Memory Engine for AI Coding Agents\n"

# 1. Detect OS
OS="$(uname -s)"
case "$OS" in
    Linux)
        PLATFORM="unknown-linux-gnu"
        ;;
    Darwin)
        PLATFORM="apple-darwin"
        ;;
    *)
        echo -e "${RED}Error: Unsupported operating system '$OS'.${RESET}" >&2
        exit 1
        ;;
esac

# 2. Detect Architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64)
        TARGET_ARCH="x86_64"
        ;;
    arm64|aarch64)
        TARGET_ARCH="aarch64"
        ;;
    *)
        echo -e "${RED}Error: Unsupported architecture '$ARCH'.${RESET}" >&2
        exit 1
        ;;
esac

TARGET="${TARGET_ARCH}-${PLATFORM}"
echo -e "Detected target: ${BOLD}${TARGET}${RESET}"

# 3. Determine Latest Version from GitHub API
echo "Fetching latest release tag from GitHub..."
RELEASE_JSON=$(curl -sSL -H "Accept: application/vnd.github.v3+json" "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true)
TAG=$(echo "$RELEASE_JSON" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' | head -n 1)

if [ -z "$TAG" ]; then
    TAG="v0.1.1"
    echo -e "${YELLOW}Could not detect latest tag via API (rate-limited?). Using fallback: ${TAG}${RESET}"
else
    echo -e "Latest release tag: ${BOLD}${TAG}${RESET}"
fi

# 4. Download and Extract Asset
PKG_NAME="strata-${TAG}-${TARGET}"
URL="https://github.com/${REPO}/releases/download/${TAG}/${PKG_NAME}.tar.gz"
TEMP_DIR=$(mktemp -d 2>/dev/null || mktemp -d -t 'strata')
trap 'rm -rf "$TEMP_DIR"' EXIT

echo -e "Downloading ${BLUE}${URL}${RESET}..."
if ! curl -fsSL "$URL" -o "${TEMP_DIR}/${PKG_NAME}.tar.gz"; then
    echo -e "${RED}Failed to download release binary for ${TARGET}.${RESET}" >&2
    echo "You can build directly from source via: cargo install strata-cli" >&2
    exit 1
fi

echo "Extracting archive..."
tar -xzf "${TEMP_DIR}/${PKG_NAME}.tar.gz" -C "$TEMP_DIR"

# 5. Determine Installation Directory
INSTALL_DIR=""
if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
elif [ -d "$HOME/.cargo/bin" ] && [ -w "$HOME/.cargo/bin" ]; then
    INSTALL_DIR="$HOME/.cargo/bin"
elif [ -d "$HOME/.local/bin" ] && [ -w "$HOME/.local/bin" ]; then
    INSTALL_DIR="$HOME/.local/bin"
else
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

# 6. Install Binary
if [ -f "${TEMP_DIR}/${PKG_NAME}/strata" ]; then
    cp "${TEMP_DIR}/${PKG_NAME}/strata" "${INSTALL_DIR}/strata"
elif [ -f "${TEMP_DIR}/strata" ]; then
    cp "${TEMP_DIR}/strata" "${INSTALL_DIR}/strata"
else
    FOUND_BIN=$(find "$TEMP_DIR" -type f -name "strata" | head -n 1)
    if [ -n "$FOUND_BIN" ]; then
        cp "$FOUND_BIN" "${INSTALL_DIR}/strata"
    else
        echo -e "${RED}Error: Could not locate strata binary in downloaded archive.${RESET}" >&2
        exit 1
    fi
fi

chmod +x "${INSTALL_DIR}/strata"
echo -e "\n${GREEN}${BOLD}✓ Strata installed successfully to ${INSTALL_DIR}/strata${RESET}"

# 7. Check PATH
case ":$PATH:" in
    *:"$INSTALL_DIR":*) ;;
    *)
        echo -e "\n${YELLOW}${BOLD}Notice:${RESET} ${INSTALL_DIR} is not in your current PATH."
        echo "To use 'strata' directly from your shell, add this to your profile (~/.bashrc, ~/.zshrc):"
        echo -e "  ${BOLD}export PATH=\"\$PATH:${INSTALL_DIR}\"${RESET}"
        export PATH="$PATH:${INSTALL_DIR}"
        ;;
esac

# 8. Run strata --version and print welcome message
echo ""
if command -v "${INSTALL_DIR}/strata" >/dev/null 2>&1; then
    "${INSTALL_DIR}/strata" --version
fi

echo -e "\n${BOLD}Welcome to Strata!${RESET}"
echo "To initialize persistent memory in your repository, run:"
echo -e "  ${GREEN}${BOLD}strata init${RESET}"
echo ""
echo "Next steps:"
echo "  strata mcp install    # Auto-configure Cursor, Claude Code, and Windsurf"
echo "  strata doctor         # Run diagnostic health check on local engine"
echo ""

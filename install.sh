#!/usr/bin/env bash
# Strata Universal Installer for macOS and Linux
# Usage: curl -fsSL https://raw.githubusercontent.com/phfarath/Strata/main/install.sh | sh

set -e

REPO="phfarath/Strata"
BOLD="\033[1m"
GREEN="\033[0;32m"
BLUE="\033[0;34m"
RED="\033[0;31m"
RESET="\033[0m"

echo -e "${BLUE}${BOLD}"
echo "  ___ _             _        "
echo " / __| |_ _ _ __ _| |_ __ _ "
echo " \__ \  _| '_/ _` |  _/ _` |"
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
        echo -e "${RED}Error: Unsupported operating system '$OS'.${RESET}"
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
        echo -e "${RED}Error: Unsupported architecture '$ARCH'.${RESET}"
        exit 1
        ;;
esac

TARGET="${TARGET_ARCH}-${PLATFORM}"
echo -e "Detected target: ${BOLD}${TARGET}${RESET}"

# 3. Determine Latest Version
echo "Fetching latest release..."
TAG=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$TAG" ]; then
    TAG="v0.1.0"
    echo "Using default release tag: ${TAG}"
else
    echo -e "Latest release tag: ${BOLD}${TAG}${RESET}"
fi

# 4. Download and Extract Asset
PKG_NAME="strata-${TAG}-${TARGET}"
URL="https://github.com/${REPO}/releases/download/${TAG}/${PKG_NAME}.tar.gz"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo -e "Downloading ${BLUE}${URL}${RESET}..."
if ! curl -fsSL "$URL" -o "${TEMP_DIR}/${PKG_NAME}.tar.gz"; then
    echo -e "${RED}Failed to download release binary for ${TARGET}.${RESET}"
    echo "You can build directly from source via: cargo install strata-cli"
    exit 1
fi

echo "Extracting archive..."
tar -xzf "${TEMP_DIR}/${PKG_NAME}.tar.gz" -C "$TEMP_DIR"

# 5. Install Binary
INSTALL_DIR="/usr/local/bin"
if [ ! -w "$INSTALL_DIR" ]; then
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

cp "${TEMP_DIR}/${PKG_NAME}/strata" "${INSTALL_DIR}/strata"
chmod +x "${INSTALL_DIR}/strata"

echo -e "\n${GREEN}${BOLD}✓ Strata installed successfully to ${INSTALL_DIR}/strata${RESET}"

# 6. Check PATH
case ":$PATH:" in
    *:"$INSTALL_DIR":*) ;;
    *)
        echo -e "\n${BOLD}Note:${RESET} ${INSTALL_DIR} is not currently in your PATH."
        echo "Add it to your shell profile (~/.bashrc, ~/.zshrc):"
        echo "  export PATH=\"\$PATH:${INSTALL_DIR}\""
        ;;
esac

echo -e "\nQuickstart:"
echo "  strata --version"
echo "  strata mcp install    # Configure Cursor, Claude Desktop, and Windsurf"
echo "  strata init           # Initialize in current project"
echo ""

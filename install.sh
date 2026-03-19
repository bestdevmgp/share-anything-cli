#!/bin/sh
set -e

REPO="bestdevmgp/share-anything-cli"
BINARY_NAME="sa"
INSTALL_DIR="/usr/local/bin"

# Detect OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$ARCH" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

case "$OS" in
    linux) TARGET="${BINARY_NAME}-linux-${ARCH}" ;;
    darwin) TARGET="${BINARY_NAME}-macos-${ARCH}" ;;
    *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

# Get latest release
LATEST_URL="https://api.github.com/repos/${REPO}/releases/latest"
echo "Fetching latest release..."

DOWNLOAD_URL=$(curl -fsSL "$LATEST_URL" | grep "browser_download_url.*${TARGET}" | head -1 | cut -d '"' -f 4)

if [ -z "$DOWNLOAD_URL" ]; then
    echo "Error: Could not find release for ${TARGET}"
    exit 1
fi

# Download and install
echo "Downloading ${TARGET}..."
TMPDIR=$(mktemp -d)
curl -fsSL "$DOWNLOAD_URL" -o "${TMPDIR}/${BINARY_NAME}"
chmod +x "${TMPDIR}/${BINARY_NAME}"

if [ -w "$INSTALL_DIR" ]; then
    mv "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
else
    echo "Installing to ${INSTALL_DIR} (requires sudo)..."
    sudo mv "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
fi

rm -rf "$TMPDIR"

echo ""
echo "✓ ${BINARY_NAME} installed to ${INSTALL_DIR}/${BINARY_NAME}"
echo "  Run 'sa --help' to get started."
echo ""

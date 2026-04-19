#!/bin/sh
# install.sh — download and install the latest vivo binary from GitHub Releases
set -e

REPO="dantuck/vivo"
INSTALL_DIR="${VIVO_INSTALL_DIR:-/usr/local/bin}"

# Detect OS and architecture
OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64)          TARGET="x86_64-unknown-linux-musl" ;;
      aarch64|arm64)   TARGET="aarch64-unknown-linux-musl" ;;
      *) echo "error: unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      *) echo "error: unsupported macOS architecture: $ARCH (only Apple Silicon supported)" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "error: unsupported OS: $OS" >&2
    exit 1
    ;;
esac

# Compute SHA256 of a file; prints the hex digest only
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo ""
  fi
}

HAS_SHA256=true
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
  echo "warning: no sha256 tool found, skipping checksum verification" >&2
  HAS_SHA256=false
fi

# Fetch latest release version (includes pre-releases since 0.x may be flagged as such)
echo "Fetching latest vivo release..."
VERSION=$(curl -sSf "https://api.github.com/repos/${REPO}/releases" \
  | grep '"tag_name"' \
  | head -1 \
  | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\([^"]*\)".*/\1/')

if [ -z "$VERSION" ] || ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+'; then
  echo "error: could not determine latest version" >&2
  exit 1
fi

ASSET="vivo-v${VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ASSET}"
CHECKSUMS_URL="https://github.com/${REPO}/releases/download/v${VERSION}/checksums.sha256"
TMP_DIR=$(mktemp -d)

echo "Downloading vivo v${VERSION} for ${TARGET}..."
curl -sSfL "$DOWNLOAD_URL" -o "${TMP_DIR}/${ASSET}"

# Verify checksum if possible
if [ "$HAS_SHA256" = "true" ]; then
  curl -sSfL "$CHECKSUMS_URL" -o "${TMP_DIR}/checksums.sha256"
  EXPECTED=$(grep "${ASSET}$" "${TMP_DIR}/checksums.sha256" | awk '{print $1}')
  ACTUAL=$(sha256_of "${TMP_DIR}/${ASSET}")
  if [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "error: checksum mismatch for $ASSET" >&2
    echo "  expected: $EXPECTED" >&2
    echo "  actual:   $ACTUAL" >&2
    rm -rf "$TMP_DIR"
    exit 1
  fi
  echo "Checksum verified."
fi

# Extract binary
tar -xzf "${TMP_DIR}/${ASSET}" -C "$TMP_DIR"

# Install
if [ -w "$INSTALL_DIR" ]; then
  mv "${TMP_DIR}/vivo" "${INSTALL_DIR}/vivo"
  chmod +x "${INSTALL_DIR}/vivo"
else
  echo "Installing to ${INSTALL_DIR} requires elevated permissions..."
  sudo mv "${TMP_DIR}/vivo" "${INSTALL_DIR}/vivo"
  sudo chmod +x "${INSTALL_DIR}/vivo"
fi

rm -rf "$TMP_DIR"

echo "vivo v${VERSION} installed to ${INSTALL_DIR}/vivo"
echo "Run 'vivo init' to get started."

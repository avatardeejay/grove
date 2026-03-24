#!/bin/sh
set -e

REPO="YourUsername/grove"
BINARY="grove"
INSTALL_DIR="/usr/local/bin"

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Linux)  OS="linux" ;;
  Darwin) OS="darwin" ;;
  *)
    echo "Unsupported OS: $OS"
    echo "Please download manually from: https://github.com/$REPO/releases"
    exit 1
    ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)          ARCH="x86_64" ;;
  aarch64 | arm64) ARCH="aarch64" ;;
  *)
    echo "Unsupported architecture: $ARCH"
    echo "Please download manually from: https://github.com/$REPO/releases"
    exit 1
    ;;
esac

TARGET="grove-${OS}-${ARCH}"
URL="https://github.com/$REPO/releases/latest/download/$TARGET"

echo "Detected: $OS / $ARCH"
echo "Downloading grove from $URL..."

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "/tmp/$BINARY"
elif command -v wget >/dev/null 2>&1; then
  wget -q "$URL" -O "/tmp/$BINARY"
else
  echo "Error: neither curl nor wget found."
  exit 1
fi

chmod +x "/tmp/$BINARY"

if [ -w "$INSTALL_DIR" ]; then
  mv "/tmp/$BINARY" "$INSTALL_DIR/$BINARY"
else
  echo "Installing to $INSTALL_DIR requires sudo..."
  sudo mv "/tmp/$BINARY" "$INSTALL_DIR/$BINARY"
fi

echo "grove installed! Run: grove --version"

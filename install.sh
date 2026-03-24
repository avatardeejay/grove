#!/bin/sh
set -e

REPO="avatardeejay/grove"

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

echo "Detected: $OS / $ARCH"

if command -v curl >/dev/null 2>&1; then
  FETCH="curl -fsSL"
elif command -v wget >/dev/null 2>&1; then
  FETCH="wget -q -O -"
else
  echo "Error: neither curl nor wget found."
  exit 1
fi

if [ "$OS" = "linux" ]; then
  URL="https://github.com/$REPO/releases/latest/download/grove-linux-$ARCH"
  echo "Downloading grove installer..."
  $FETCH "$URL" -o /tmp/grove-installer
  chmod +x /tmp/grove-installer
  /tmp/grove-installer

elif [ "$OS" = "darwin" ]; then
  URL="https://github.com/$REPO/releases/latest/download/grove-darwin-$ARCH.zip"
  echo "Downloading grove installer..."
  $FETCH "$URL" -o /tmp/grove-installer.zip
  unzip -q /tmp/grove-installer.zip -d /tmp/grove-installer-mac
  APP=$(find /tmp/grove-installer-mac -name "*.app" | head -1)
  open "$APP"
fi

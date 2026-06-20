#!/bin/bash
set -e

INSTALL_DIR="${CONDUIT_INSTALL_DIR:-$HOME/.local/bin}"

# Build the web UI first so the daemon embeds the current assets. Skipped (with
# a warning) when bun isn't installed — the binary then serves a placeholder.
if command -v bun >/dev/null 2>&1; then
  echo "Building web UI..."
  ( cd web && bun install --silent && bun run build )
else
  echo "Note: bun not found — skipping web UI build; the embedded UI will be a placeholder."
fi

echo "Building conduit (release)..."
cargo build --release -p tui

VERSION=$(cargo metadata --no-deps --format-version 1 | grep -o '"version":"[^"]*"' | head -1 | cut -d'"' -f4)

mkdir -p "$INSTALL_DIR"
rm -f "$INSTALL_DIR/conduit"
cp target/release/conduit "$INSTALL_DIR/conduit"
chmod +x "$INSTALL_DIR/conduit"

echo "Installed conduit ${VERSION} to ${INSTALL_DIR}/conduit"

case ":$PATH:" in
  *":${INSTALL_DIR}:"*) ;;
  *) echo "Note: ${INSTALL_DIR} is not in your PATH. Add it with:"
     echo "  export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac

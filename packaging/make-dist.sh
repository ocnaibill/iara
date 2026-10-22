#!/usr/bin/env bash
# ==============================================================================
# make-dist.sh - Cria o arquivo tarball de release do Iara (x86_64 Linux)
# ==============================================================================
set -euo pipefail

VERSION="${1:-0.1.0}"
ARCH="x86_64"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
PKG_NAME="iara-v${VERSION}-${ARCH}-linux"
STAGE_DIR="$ROOT_DIR/target/$PKG_NAME"

echo "=== Construindo pacote de distribuição: ${PKG_NAME}.tar.gz ==="

# Garante binários compilados
if [ ! -f "$ROOT_DIR/target/release/iara-service" ] || [ ! -f "$ROOT_DIR/target/release/iara-ui" ]; then
  echo "Compilando binários em modo release..."
  cargo build --release
fi

rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/data" "$DIST_DIR"

# Copia binários
install -m 755 "$ROOT_DIR/target/release/iara-service" "$STAGE_DIR/bin/iara-service"
install -m 755 "$ROOT_DIR/target/release/iara-ui" "$STAGE_DIR/bin/iara-ui"

# Copia dados e metadados
cp -r "$ROOT_DIR/data"/* "$STAGE_DIR/data/"

# Copia instalador e documentação
install -m 755 "$ROOT_DIR/packaging/install.sh" "$STAGE_DIR/install.sh"
cp "$ROOT_DIR/LICENSE" "$STAGE_DIR/LICENSE"
cp "$ROOT_DIR/README.md" "$STAGE_DIR/README.md"
cp "$ROOT_DIR/CHANGELOG.md" "$STAGE_DIR/CHANGELOG.md"

# Cria Makefile minimalista para quem prefere rodar `make install`
cat <<'EOF' > "$STAGE_DIR/Makefile"
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
SYSTEMD_USER_DIR ?= $(PREFIX)/lib/systemd/user

.PHONY: install install-bin uninstall

install: install-bin

install-bin:
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 755 bin/iara-service "$(DESTDIR)$(BINDIR)/iara-service"
	install -m 755 bin/iara-ui "$(DESTDIR)$(BINDIR)/iara-ui"
	install -d "$(DESTDIR)$(DATADIR)/applications"
	install -m 644 data/dev.iara.Mixer.desktop "$(DESTDIR)$(DATADIR)/applications/dev.iara.Mixer.desktop"
	install -d "$(DESTDIR)$(DATADIR)/metainfo"
	install -m 644 data/dev.iara.Mixer.metainfo.xml "$(DESTDIR)$(DATADIR)/metainfo/dev.iara.Mixer.metainfo.xml"
	install -d "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps"
	install -m 644 data/icons/hicolor/scalable/apps/dev.iara.Mixer.svg "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/dev.iara.Mixer.svg"
	install -d "$(DESTDIR)$(SYSTEMD_USER_DIR)"
	install -m 644 data/systemd/user/iara.service "$(DESTDIR)$(SYSTEMD_USER_DIR)/iara.service"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/iara-service"
	rm -f "$(DESTDIR)$(BINDIR)/iara-ui"
	rm -f "$(DESTDIR)$(DATADIR)/applications/dev.iara.Mixer.desktop"
	rm -f "$(DESTDIR)$(DATADIR)/metainfo/dev.iara.Mixer.metainfo.xml"
	rm -f "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/dev.iara.Mixer.svg"
	rm -f "$(DESTDIR)$(SYSTEMD_USER_DIR)/iara.service"
EOF

# Empacota tar.gz
TARBALL="$DIST_DIR/${PKG_NAME}.tar.gz"
(
  cd "$ROOT_DIR/target"
  tar --numeric-owner --owner=0 --group=0 -czf "$TARBALL" "$PKG_NAME"
)

# Gera SHA256 checksum
(
  cd "$DIST_DIR"
  sha256sum "$(basename "$TARBALL")" > "${TARBALL}.sha256"
)

rm -rf "$STAGE_DIR"
echo "Tarball gerado com sucesso: $TARBALL"
ls -lh "$TARBALL"*

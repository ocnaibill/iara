#!/usr/bin/env bash
# ==============================================================================
# make-deb.sh - Constrói pacote Debian (.deb) para o Iara
# ==============================================================================
set -euo pipefail

VERSION="${1:-0.1.0}"
ARCH="amd64"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="$ROOT_DIR/target/deb-build"
DIST_DIR="$ROOT_DIR/dist"
PKG_NAME="iara_${VERSION}_${ARCH}"

echo "=== Construindo pacote Debian: ${PKG_NAME}.deb ==="

# Garante que os binários existem
if [ ! -f "$ROOT_DIR/target/release/iara-service" ] || [ ! -f "$ROOT_DIR/target/release/iara-ui" ]; then
  echo "Compilando binários em modo release..."
  cargo build --workspace --release --all-features
fi

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/debian" "$BUILD_DIR/data" "$DIST_DIR"

# 1. Estrutura do sistema de arquivos (data)
PREFIX="$BUILD_DIR/data/usr"
mkdir -p "$PREFIX/bin" \
         "$PREFIX/share/applications" \
         "$PREFIX/share/metainfo" \
         "$PREFIX/share/icons/hicolor/scalable/apps" \
         "$PREFIX/share/doc/iara" \
         "$PREFIX/lib/systemd/user"

install -m 755 "$ROOT_DIR/target/release/iara-service" "$PREFIX/bin/iara-service"
install -m 755 "$ROOT_DIR/target/release/iara-ui" "$PREFIX/bin/iara-ui"

install -m 644 "$ROOT_DIR/data/dev.iara.Mixer.desktop" "$PREFIX/share/applications/dev.iara.Mixer.desktop"
install -m 644 "$ROOT_DIR/data/dev.iara.Mixer.metainfo.xml" "$PREFIX/share/metainfo/dev.iara.Mixer.metainfo.xml"
install -m 644 "$ROOT_DIR/data/icons/hicolor/scalable/apps/dev.iara.Mixer.svg" "$PREFIX/share/icons/hicolor/scalable/apps/dev.iara.Mixer.svg"
install -m 644 "$ROOT_DIR/data/systemd/user/iara.service" "$PREFIX/lib/systemd/user/iara.service"

install -m 644 "$ROOT_DIR/LICENSE" "$PREFIX/share/doc/iara/copyright"
gzip -9cn "$ROOT_DIR/CHANGELOG.md" > "$PREFIX/share/doc/iara/changelog.Debian.gz"

# 2. Arquivos de controle (debian)
cat <<EOF > "$BUILD_DIR/debian/control"
Package: iara
Version: ${VERSION}
Section: sound
Priority: optional
Architecture: ${ARCH}
Maintainer: ocnaibill <biancocosta01@gmail.com>
Homepage: https://github.com/ocnaibill/iara
Depends: pipewire (>= 0.3.0), wireplumber, libgtk-4-1, libglib2.0-0, libc6
Description: Advanced submixing and audio routing for Linux with PipeWire
 Iara is an advanced audio mixer and router for Linux powered by PipeWire.
 It splits apps across dedicated virtual audio channels (Media, Games, Chat,
 Aux, Browser) and creates distinct mixes for headphones and stream.
EOF

cat <<'EOF' > "$BUILD_DIR/debian/postinst"
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q /usr/share/applications || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
    fi
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload 2>/dev/null || true
    fi
fi
exit 0
EOF
chmod 755 "$BUILD_DIR/debian/postinst"

cat <<'EOF' > "$BUILD_DIR/debian/postrm"
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload 2>/dev/null || true
fi
exit 0
EOF
chmod 755 "$BUILD_DIR/debian/postrm"

# Cria md5sums
(
  cd "$BUILD_DIR/data"
  find . -type f ! -path './DEBIAN/*' -exec md5sum {} + | sed -e 's| \./| |' > "$BUILD_DIR/debian/md5sums"
)

# 3. Compactação compatível com dpkg
echo "2.0" > "$BUILD_DIR/debian-binary"

(
  cd "$BUILD_DIR/debian"
  tar --numeric-owner --owner=0 --group=0 -czf "$BUILD_DIR/control.tar.gz" .
)

(
  cd "$BUILD_DIR/data"
  tar --numeric-owner --owner=0 --group=0 -czf "$BUILD_DIR/data.tar.gz" .
)

# 4. Cria arquivo .deb com ar
(
  cd "$BUILD_DIR"
  ar -rcs "$DIST_DIR/${PKG_NAME}.deb" debian-binary control.tar.gz data.tar.gz
)

rm -rf "$BUILD_DIR"
echo "Pacote Debian gerado com sucesso: $DIST_DIR/${PKG_NAME}.deb"
ls -lh "$DIST_DIR/${PKG_NAME}.deb"

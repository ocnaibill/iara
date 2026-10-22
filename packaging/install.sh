#!/usr/bin/env bash
# ==============================================================================
# install.sh - Instalador simples para o Iara (Mixer de Áudio Linux)
# ==============================================================================
set -euo pipefail

VERSION="0.1.0"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$SCRIPT_DIR"
if [ ! -d "$ROOT_DIR/data" ] && [ -d "$ROOT_DIR/../data" ]; then
  ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

# Encontra binários pré-compilados
if [ -f "$ROOT_DIR/bin/iara-service" ] && [ -f "$ROOT_DIR/bin/iara-ui" ]; then
  BIN_SRC="$ROOT_DIR/bin"
elif [ -f "$ROOT_DIR/target/release/iara-service" ] && [ -f "$ROOT_DIR/target/release/iara-ui" ]; then
  BIN_SRC="$ROOT_DIR/target/release"
elif [ -f "$ROOT_DIR/iara-service" ] && [ -f "$ROOT_DIR/iara-ui" ]; then
  BIN_SRC="$ROOT_DIR"
else
  echo "Erro: Binários 'iara-service' e 'iara-ui' não encontrados."
  echo "Execute 'cargo build --release' antes de instalar."
  exit 1
fi

DATA_SRC="$ROOT_DIR/data"
if [ ! -d "$DATA_SRC" ] && [ -d "$ROOT_DIR/../data" ]; then
  DATA_SRC="$ROOT_DIR/../data"
fi

MODE="auto"
ACTION="install"

for arg in "$@"; do
  case "$arg" in
    --user)
      MODE="user"
      ;;
    --system)
      MODE="system"
      ;;
    --uninstall)
      ACTION="uninstall"
      ;;
    -h|--help)
      echo "Uso: $0 [OPÇÕES]"
      echo ""
      echo "Opções:"
      echo "  --user        Instala apenas para o usuário atual (~/.local)"
      echo "  --system      Instala para todo o sistema (/usr/local, requer sudo)"
      echo "  --uninstall   Remove os arquivos instalados"
      echo "  -h, --help    Exibe esta ajuda"
      exit 0
      ;;
    *)
      echo "Opção desconhecida: $arg"
      exit 1
      ;;
  esac
done

if [ "$MODE" = "auto" ]; then
  if [ "$(id -u)" -eq 0 ]; then
    MODE="system"
  else
    MODE="user"
  fi
fi

if [ "$MODE" = "system" ]; then
  PREFIX="/usr/local"
  SYSTEMD_DIR="/usr/local/lib/systemd/user"
else
  PREFIX="$HOME/.local"
  SYSTEMD_DIR="$HOME/.config/systemd/user"
fi

BINDIR="$PREFIX/bin"
DATADIR="$PREFIX/share"
APPSDIR="$DATADIR/applications"
ICONDIR="$DATADIR/icons/hicolor/scalable/apps"
METAINFODIR="$DATADIR/metainfo"

if [ "$ACTION" = "uninstall" ]; then
  echo "Removendo Iara de $PREFIX..."
  rm -f "$BINDIR/iara-service" "$BINDIR/iara-ui"
  rm -f "$APPSDIR/dev.iara.Mixer.desktop"
  rm -f "$METAINFODIR/dev.iara.Mixer.metainfo.xml"
  rm -f "$ICONDIR/dev.iara.Mixer.svg"
  rm -f "$SYSTEMD_DIR/iara.service"
  echo "Desinstalação concluída!"
  exit 0
fi

echo "=== Instalando Iara v$VERSION ($MODE: $PREFIX) ==="

mkdir -p "$BINDIR" "$APPSDIR" "$ICONDIR" "$METAINFODIR" "$SYSTEMD_DIR"

install -m 755 "$BIN_SRC/iara-service" "$BINDIR/iara-service"
install -m 755 "$BIN_SRC/iara-ui" "$BINDIR/iara-ui"

install -m 644 "$DATA_SRC/dev.iara.Mixer.desktop" "$APPSDIR/dev.iara.Mixer.desktop"
install -m 644 "$DATA_SRC/dev.iara.Mixer.metainfo.xml" "$METAINFODIR/dev.iara.Mixer.metainfo.xml"
install -m 644 "$DATA_SRC/icons/hicolor/scalable/apps/dev.iara.Mixer.svg" "$ICONDIR/dev.iara.Mixer.svg"
install -m 644 "$DATA_SRC/systemd/user/iara.service" "$SYSTEMD_DIR/iara.service"
sed -i "s|ExecStart=.*|ExecStart=$BINDIR/iara-service|" "$SYSTEMD_DIR/iara.service"

# Atualização de caches do desktop se disponíveis
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APPSDIR" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f "$DATADIR/icons/hicolor" 2>/dev/null || true
fi
if command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload 2>/dev/null || true
fi

echo ""
echo "Instalação concluída com sucesso!"
echo "Binários instalados em: $BINDIR"
echo ""
echo "Para habilitar e iniciar o Iara automaticamente na sessão:"
echo "  systemctl --user enable --now iara.service"
echo ""
echo "Para abrir a interface do mixer:"
echo "  iara-ui"

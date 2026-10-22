#!/usr/bin/env bash
# ==============================================================================
# setup.sh - Instalação e verificação de pré-requisitos para o Iara
# ==============================================================================
set -euo pipefail

# Cores para saída no terminal
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # Sem cor

# Flags e parâmetros
INCLUDE_TEST_DEPS=false
CHECK_ONLY=false
NON_INTERACTIVE=false
SKIP_RUST=false
RUN_BUILD_CHECK=false

print_help() {
  cat <<EOF
Uso: $0 [OPÇÕES]

Configura o ambiente e instala as dependências necessárias para compilar e rodar o Iara.

Opções:
  -a, --all, --with-tests   Instala também ferramentas para testes e provas (sox, xdotool, xvfb, openbox).
  -c, --check               Apenas verifica os requisitos sem instalar nada.
  -y, --yes                 Modo não interativo: assume 'sim' para todas as perguntas.
  --no-rust                 Pula a instalação automática do Rust / rustup.
  --build-check             Executa verificação de compilação (cargo check) ao finalizar.
  -h, --help                Exibe esta mensagem de ajuda.

EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -a|--all|--with-tests)
      INCLUDE_TEST_DEPS=true
      shift
      ;;
    -c|--check)
      CHECK_ONLY=true
      shift
      ;;
    -y|--yes)
      NON_INTERACTIVE=true
      shift
      ;;
    --no-rust)
      SKIP_RUST=true
      shift
      ;;
    --build-check)
      RUN_BUILD_CHECK=true
      shift
      ;;
    -h|--help)
      print_help
      exit 0
      ;;
    *)
      echo -e "${RED}Opção desconhecida: $1${NC}"
      print_help
      exit 1
      ;;
  esac
done

echo -e "${BOLD}${BLUE}=== Verificação e Instalação de Requisitos do Iara ===${NC}\n"

# Carrega ambiente do Cargo se já existir em ~/.cargo/env
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi

# 1. Identificar a distribuição Linux
DISTRO_ID=""
DISTRO_LIKE=""
if [ -f /etc/os-release ]; then
  # shellcheck source=/dev/null
  . /etc/os-release
  DISTRO_ID="${ID:-}"
  DISTRO_LIKE="${ID_LIKE:-}"
fi

echo -e "Sistema operacional detectado: ${BOLD}${PRETTY_NAME:-Linux}${NC}"

# Função auxiliar para checar pacotes instalados via pkg-config
has_pkg() {
  pkg-config --exists "$1" 2>/dev/null
}

# Função auxiliar para checar binário no PATH
has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

# 2. Diagnóstico dos componentes
MISSING_SYS_PKGS=()
MISSING_TEST_PKGS=()

# Checagem de Rust
HAS_RUST=false
if has_cmd cargo && has_cmd rustc; then
  HAS_RUST=true
fi

# Checagem de ferramentas básicas
HAS_PKG_CONFIG=false
if has_cmd pkg-config; then HAS_PKG_CONFIG=true; fi

HAS_CC=false
if has_cmd gcc || has_cmd clang; then HAS_CC=true; fi

HAS_CLANG=false
if has_cmd clang; then HAS_CLANG=true; fi

HAS_LIBPIPEWIRE=false
if has_pkg libpipewire-0.3; then HAS_LIBPIPEWIRE=true; fi

HAS_GTK4=false
if has_pkg gtk4; then HAS_GTK4=true; fi

HAS_PIPEWIRE_CMD=false
if has_cmd pipewire; then HAS_PIPEWIRE_CMD=true; fi

HAS_WIREPLUMBER=false
if has_cmd wireplumber; then HAS_WIREPLUMBER=true; fi

# Dependências de teste / medições
HAS_SOX=false
if has_cmd sox; then HAS_SOX=true; fi

HAS_XDOTOOL=false
if has_cmd xdotool; then HAS_XDOTOOL=true; fi

HAS_XVFB=false
if has_cmd Xvfb || has_cmd xvfb-run; then HAS_XVFB=true; fi

HAS_OPENBOX=false
if has_cmd openbox; then HAS_OPENBOX=true; fi

HAS_PYGOBJECT=false
if python3 -c "import gi" >/dev/null 2>&1; then HAS_PYGOBJECT=true; fi

# Exibir status
echo -e "\n${BOLD}Diagnóstico de Requisitos Principais:${NC}"
echo -e "  Rust & Cargo:          $([ "$HAS_RUST" = true ] && echo -e "${GREEN}OK ($(rustc --version 2>/dev/null | cut -d' ' -f2))${NC}" || echo -e "${RED}Ausente${NC}")"
echo -e "  Compilador C (gcc/clang): $([ "$HAS_CC" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${RED}Ausente${NC}")"
echo -e "  pkg-config:            $([ "$HAS_PKG_CONFIG" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${RED}Ausente${NC}")"
echo -e "  Clang / libclang:      $([ "$HAS_CLANG" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${RED}Ausente (necessário para bindgen do PipeWire)${NC}")"
echo -e "  libpipewire-0.3:       $([ "$HAS_LIBPIPEWIRE" = true ] && echo -e "${GREEN}OK ($(pkg-config --modversion libpipewire-0.3 2>/dev/null))${NC}" || echo -e "${RED}Ausente${NC}")"
echo -e "  GTK 4 (devel):         $([ "$HAS_GTK4" = true ] && echo -e "${GREEN}OK ($(pkg-config --modversion gtk4 2>/dev/null))${NC}" || echo -e "${RED}Ausente${NC}")"
echo -e "  PipeWire daemon:       $([ "$HAS_PIPEWIRE_CMD" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${YELLOW}Não encontrado no PATH${NC}")"
echo -e "  WirePlumber:           $([ "$HAS_WIREPLUMBER" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${YELLOW}Não encontrado no PATH${NC}")"

echo -e "\n${BOLD}Diagnóstico de Ferramentas de Testes e Provas (Opcionais):${NC}"
echo -e "  sox (medições RMS):    $([ "$HAS_SOX" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${YELLOW}Ausente${NC}")"
echo -e "  xdotool (UI entrada):  $([ "$HAS_XDOTOOL" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${YELLOW}Ausente${NC}")"
echo -e "  Xvfb (display virtual):$([ "$HAS_XVFB" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${YELLOW}Ausente${NC}")"
echo -e "  openbox (display virt):$([ "$HAS_OPENBOX" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${YELLOW}Ausente${NC}")"
echo -e "  python-gobject (a11y): $([ "$HAS_PYGOBJECT" = true ] && echo -e "${GREEN}OK${NC}" || echo -e "${YELLOW}Ausente${NC}")"

# Se check-only, finalizar aqui
if [ "$CHECK_ONLY" = true ]; then
  echo -e "\n${BLUE}Verificação concluída (--check). Nenhuma alteração foi feita.${NC}"
  exit 0
fi

# Determinar comandos por gerenciador de pacotes
PKG_MANAGER=""
INSTALL_CMD=()
SYS_PKGS=()
TEST_PKGS=()

if [[ "$DISTRO_ID" =~ ^(arch|cachyos|manjaro|endeavouros)$ ]] || [[ "$DISTRO_LIKE" =~ arch ]]; then
  PKG_MANAGER="pacman"
  INSTALL_CMD=(sudo pacman -S --needed)
  [ "$HAS_CC" = false ] && SYS_PKGS+=(base-devel)
  [ "$HAS_PKG_CONFIG" = false ] && SYS_PKGS+=(pkgconf)
  [ "$HAS_CLANG" = false ] && SYS_PKGS+=(clang)
  [ "$HAS_LIBPIPEWIRE" = false ] && SYS_PKGS+=(pipewire)
  [ "$HAS_WIREPLUMBER" = false ] && SYS_PKGS+=(wireplumber)
  [ "$HAS_GTK4" = false ] && SYS_PKGS+=(gtk4)
  
  [ "$HAS_SOX" = false ] && TEST_PKGS+=(sox)
  [ "$HAS_XDOTOOL" = false ] && TEST_PKGS+=(xdotool)
  [ "$HAS_XVFB" = false ] && TEST_PKGS+=(xorg-server-xvfb)
  [ "$HAS_OPENBOX" = false ] && TEST_PKGS+=(openbox)
  [ "$HAS_PYGOBJECT" = false ] && TEST_PKGS+=(python-gobject)

elif [[ "$DISTRO_ID" =~ ^(ubuntu|debian|linuxmint|pop)$ ]] || [[ "$DISTRO_LIKE" =~ (ubuntu|debian) ]]; then
  PKG_MANAGER="apt"
  INSTALL_CMD=(sudo apt install -y)
  [ "$HAS_CC" = false ] && SYS_PKGS+=(build-essential)
  [ "$HAS_PKG_CONFIG" = false ] && SYS_PKGS+=(pkg-config)
  [ "$HAS_CLANG" = false ] && SYS_PKGS+=(clang libclang-dev)
  [ "$HAS_LIBPIPEWIRE" = false ] && SYS_PKGS+=(libpipewire-0.3-dev pipewire)
  [ "$HAS_WIREPLUMBER" = false ] && SYS_PKGS+=(wireplumber)
  [ "$HAS_GTK4" = false ] && SYS_PKGS+=(libgtk-4-dev)

  [ "$HAS_SOX" = false ] && TEST_PKGS+=(sox libsox-fmt-all)
  [ "$HAS_XDOTOOL" = false ] && TEST_PKGS+=(xdotool)
  [ "$HAS_XVFB" = false ] && TEST_PKGS+=(xvfb)
  [ "$HAS_OPENBOX" = false ] && TEST_PKGS+=(openbox)
  [ "$HAS_PYGOBJECT" = false ] && TEST_PKGS+=(python3-gi)

elif [[ "$DISTRO_ID" =~ ^(fedora|rhel|centos)$ ]] || [[ "$DISTRO_LIKE" =~ fedora ]]; then
  PKG_MANAGER="dnf"
  INSTALL_CMD=(sudo dnf install -y)
  [ "$HAS_CC" = false ] && SYS_PKGS+=("@development-tools")
  [ "$HAS_PKG_CONFIG" = false ] && SYS_PKGS+=(pkgconf-pkg-config)
  [ "$HAS_CLANG" = false ] && SYS_PKGS+=(clang clang-devel)
  [ "$HAS_LIBPIPEWIRE" = false ] && SYS_PKGS+=(pipewire-devel pipewire)
  [ "$HAS_WIREPLUMBER" = false ] && SYS_PKGS+=(wireplumber)
  [ "$HAS_GTK4" = false ] && SYS_PKGS+=(gtk4-devel)

  [ "$HAS_SOX" = false ] && TEST_PKGS+=(sox)
  [ "$HAS_XDOTOOL" = false ] && TEST_PKGS+=(xdotool)
  [ "$HAS_XVFB" = false ] && TEST_PKGS+=(xorg-x11-server-Xvfb)
  [ "$HAS_OPENBOX" = false ] && TEST_PKGS+=(openbox)
  [ "$HAS_PYGOBJECT" = false ] && TEST_PKGS+=(python3-gobject)

elif [[ "$DISTRO_ID" =~ ^(opensuse|sles)$ ]] || [[ "$DISTRO_LIKE" =~ (opensuse|suse) ]]; then
  PKG_MANAGER="zypper"
  INSTALL_CMD=(sudo zypper install -y)
  [ "$HAS_CC" = false ] && SYS_PKGS+=(patterns-devel-base-devel_basis)
  [ "$HAS_PKG_CONFIG" = false ] && SYS_PKGS+=(pkg-config)
  [ "$HAS_CLANG" = false ] && SYS_PKGS+=(clang clang-devel)
  [ "$HAS_LIBPIPEWIRE" = false ] && SYS_PKGS+=(pipewire-devel pipewire)
  [ "$HAS_WIREPLUMBER" = false ] && SYS_PKGS+=(wireplumber)
  [ "$HAS_GTK4" = false ] && SYS_PKGS+=(gtk4-devel)

  [ "$HAS_SOX" = false ] && TEST_PKGS+=(sox)
  [ "$HAS_XDOTOOL" = false ] && TEST_PKGS+=(xdotool)
  [ "$HAS_XVFB" = false ] && TEST_PKGS+=(xvfb-run)
  [ "$HAS_OPENBOX" = false ] && TEST_PKGS+=(openbox)
  [ "$HAS_PYGOBJECT" = false ] && TEST_PKGS+=(python3-gobject)
else
  echo -e "\n${YELLOW}Distribuição não reconhecida automaticamente para pacotes de sistema.${NC}"
  echo "Certifique-se de ter instalado: compilador C/C++, pkg-config, clang/libclang, libpipewire-0.3-dev e gtk4-dev."
fi

# Instalar pacotes de sistema se necessário
TOTAL_SYS_PKGS=("${SYS_PKGS[@]}")
if [ "$INCLUDE_TEST_DEPS" = true ]; then
  TOTAL_SYS_PKGS+=("${TEST_PKGS[@]}")
fi

if [ ${#TOTAL_SYS_PKGS[@]} -gt 0 ]; then
  echo -e "\n${BOLD}Pacotes do sistema que precisam ser instalados:${NC} ${TOTAL_SYS_PKGS[*]}"
  
  CONFIRM=false
  if [ "$NON_INTERACTIVE" = true ]; then
    CONFIRM=true
  else
    read -r -p "Deseja instalar estes pacotes via $PKG_MANAGER? [S/n] " response
    if [[ "$response" =~ ^([sS][iI][mM]|[sS]|"")$ ]]; then
      CONFIRM=true
    fi
  fi

  if [ "$CONFIRM" = true ]; then
    echo -e "${BLUE}Executando:${NC} ${INSTALL_CMD[*]} ${TOTAL_SYS_PKGS[*]}"
    if [ "$PKG_MANAGER" = "apt" ]; then
      sudo apt update
      sudo apt install -y "${TOTAL_SYS_PKGS[@]}"
    else
      "${INSTALL_CMD[@]}" "${TOTAL_SYS_PKGS[@]}"
    fi
  else
    echo -e "${YELLOW}Instalação de pacotes cancelada pelo usuário.${NC}"
  fi
else
  echo -e "\n${GREEN}Todos os pacotes de sistema essenciais já estão instalados!${NC}"
fi

# Instalação do Rust via rustup se necessário
if [ "$HAS_RUST" = false ] && [ "$SKIP_RUST" = false ]; then
  echo -e "\n${BOLD}Rust e Cargo não foram encontrados.${NC}"
  CONFIRM_RUST=false
  if [ "$NON_INTERACTIVE" = true ]; then
    CONFIRM_RUST=true
  else
    read -r -p "Deseja instalar o Rust oficial via rustup agora? [S/n] " r_ans
    if [[ "$r_ans" =~ ^([sS][iI][mM]|[sS]|"")$ ]]; then
      CONFIRM_RUST=true
    fi
  fi

  if [ "$CONFIRM_RUST" = true ]; then
    echo -e "${BLUE}Baixando e instalando rustup...${NC}"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    if [ -f "$HOME/.cargo/env" ]; then
      # shellcheck source=/dev/null
      . "$HOME/.cargo/env"
    fi
    rustup component add rustfmt clippy || true
    echo -e "${GREEN}Rust instalado com sucesso! ($(rustc --version))${NC}"
    HAS_RUST=true
  else
    echo -e "${YELLOW}Rust não foi instalado. Instale manualmente antes de compilar.${NC}"
  fi
fi

# 3. Verificação do serviço PipeWire da sessão
echo -e "\n${BOLD}Status do PipeWire na sessão de usuário:${NC}"
if has_cmd systemctl; then
  PW_STATUS=$(systemctl --user is-active pipewire 2>/dev/null || echo "inativo")
  WP_STATUS=$(systemctl --user is-active wireplumber 2>/dev/null || echo "inativo")

  if [ "$PW_STATUS" = "active" ]; then
    echo -e "  PipeWire daemon:       ${GREEN}Ativo (systemd --user)${NC}"
  else
    echo -e "  PipeWire daemon:       ${YELLOW}$PW_STATUS${NC}"
    echo -e "  ${YELLOW}Dica: execute 'systemctl --user enable --now pipewire' se necessário.${NC}"
  fi

  if [ "$WP_STATUS" = "active" ]; then
    echo -e "  WirePlumber:           ${GREEN}Ativo (systemd --user)${NC}"
  else
    echo -e "  WirePlumber:           ${YELLOW}$WP_STATUS${NC}"
    echo -e "  ${YELLOW}Dica: execute 'systemctl --user enable --now wireplumber' se necessário.${NC}"
  fi
fi

# 4. Verificação de compilação (opcional)
if [ "$RUN_BUILD_CHECK" = true ] && [ "$HAS_RUST" = true ]; then
  echo -e "\n${BOLD}Executando verificação do workspace Rust (cargo check)...${NC}"
  cargo check --workspace
  cargo check -p iara-ui --features gtk-ui
  echo -e "${GREEN}Workspace verificado com sucesso!${NC}"
fi

echo -e "\n${BOLD}${GREEN}=== Configuração concluída! ===${NC}"
echo -e "Comandos para iniciar e testar o Iara:"
echo -e "  ${BOLD}tools/teste-humano.sh${NC}                           # Lança serviço e janela em ambiente isolado de teste"
echo -e "  ${BOLD}cargo run -p iara-ui --features gtk-ui -- --demo${NC}     # Abre a interface com dados de demonstração"
echo -e "  ${BOLD}cargo test --workspace${NC}                               # Roda testes unitários do workspace"
echo -e ""

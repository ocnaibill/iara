# Biblioteca (use com `source`): sobe um PipeWire + WirePlumber PRIVADOS, sem dispositivos físicos e sem tocar na sessão do usuário.
# Define PIPEWIRE_RUNTIME_DIR/XDG_*/DBUS para o resto do script; chame `privado_parar` no trap de saída.
privado_subir() {
  PRIV=$(mktemp -d /tmp/iara-priv.XXXXXX)
  mkdir -p "$PRIV/rt" "$PRIV/cfg/wireplumber/wireplumber.conf.d" "$PRIV/state" "$PRIV/data"
  chmod 700 "$PRIV/rt"
  cat > "$PRIV/cfg/wireplumber/wireplumber.conf.d/90-privado.conf" <<'CONF'
wireplumber.profiles = {
  main = {
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.bluez-midi = disabled
    monitor.libcamera = disabled
    monitor.v4l2 = disabled
    support.reserve-device = disabled
  }
}
CONF
  export PIPEWIRE_RUNTIME_DIR="$PRIV/rt" XDG_CONFIG_HOME="$PRIV/cfg" XDG_STATE_HOME="$PRIV/state" XDG_DATA_HOME="$PRIV/data"
  export PIPEWIRE_REMOTE=pipewire-0
  pipewire > "$PRIV/pipewire.log" 2>&1 & PW_PID=$!
  for _ in $(seq 1 50); do [ -S "$PRIV/rt/pipewire-0" ] && break; sleep 0.1; done
  wireplumber > "$PRIV/wireplumber.log" 2>&1 & WP_PID=$!
  for _ in $(seq 1 80); do pw-cli info 0 >/dev/null 2>&1 && pw-metadata -n default >/dev/null 2>&1 && break; sleep 0.1; done
  sleep 1.5
}
# Barramento de sessão D-Bus PRIVADO (serviço e janela de teste não encontram nem ativam os reais). Chame depois de privado_subir.
privado_dbus() {
  DBUS_PID=""
  local addr
  addr=$("$(command -v /usr/bin/dbus-daemon || command -v dbus-daemon)" --session --fork --print-address=1 --print-pid=2 2> "$PRIV/dbus.pid") || return 1
  export DBUS_SESSION_BUS_ADDRESS="$addr"
  DBUS_PID=$(tr -d '[:space:]' < "$PRIV/dbus.pid")
}
# Tela X PRIVADA (Xvfb): a janela de teste não aparece na tela de ninguém, não depende de a sessão estar desbloqueada e o
# ponteiro real nunca é tocado. Se houver `openbox`, sobe como gerenciador de janelas (minimizar/restaurar de verdade).
# Requer o pacote xorg-server-xvfb (e, opcionalmente, openbox).
privado_tela() {
  command -v Xvfb >/dev/null || return 1
  Xvfb -displayfd 3 -screen 0 1600x900x24 -nolisten tcp 3> "$PRIV/display" 2> "$PRIV/xvfb.log" & XVFB_PID=$!
  for _ in $(seq 1 50); do [ -s "$PRIV/display" ] && break; sleep 0.1; done
  [ -s "$PRIV/display" ] || return 1
  export DISPLAY=":$(tr -d '[:space:]' < "$PRIV/display")" GDK_BACKEND=x11 GSK_RENDERER=cairo
  unset WAYLAND_DISPLAY
  if command -v openbox >/dev/null; then openbox > "$PRIV/openbox.log" 2>&1 & WM_PID=$!; sleep 1; fi
}
privado_parar() {
  [ -n "${WM_PID:-}" ] && kill "$WM_PID" 2>/dev/null
  [ -n "${XVFB_PID:-}" ] && kill "$XVFB_PID" 2>/dev/null
  [ -n "${DBUS_PID:-}" ] && kill "$DBUS_PID" 2>/dev/null
  [ -n "${WP_PID:-}" ] && kill "$WP_PID" 2>/dev/null
  [ -n "${PW_PID:-}" ] && kill "$PW_PID" 2>/dev/null
  wait 2>/dev/null
  [ -n "${PRIV:-}" ] && rm -rf "$PRIV"
}

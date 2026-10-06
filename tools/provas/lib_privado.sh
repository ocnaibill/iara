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
privado_parar() {
  [ -n "${WP_PID:-}" ] && kill "$WP_PID" 2>/dev/null
  [ -n "${PW_PID:-}" ] && kill "$PW_PID" 2>/dev/null
  wait 2>/dev/null
  [ -n "${PRIV:-}" ] && rm -rf "$PRIV"
}

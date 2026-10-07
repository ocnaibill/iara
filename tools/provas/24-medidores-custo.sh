#!/usr/bin/env bash
# Custo dos medidores ao vivo: CPU (tempo de CPU / tempo real) do serviço, da janela e do PipeWire em 4 situações, num PipeWire,
# D-Bus e tela privados: (a) janela minimizada (sem tap), (b) medidores ligados em silêncio, (c) medidores ligados com som,
# (d) janela minimizada com som. Também conta os pacotes `Levels` por segundo no barramento e os xruns do PipeWire privado.
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
for t in xdotool sox pw-play busctl Xvfb openbox; do command -v $t >/dev/null || { echo "falta: $t"; exit 2; }; done
# PERFIL=release mede o binário otimizado (o de debug superestima o custo)
PERFIL="${PERFIL:-debug}"
(cd "$ROOT" && cargo build -q $([ "$PERFIL" = release ] && echo --release) -p iara-service -p iara-ui --features gtk-ui) || exit 1
BIN="$ROOT/target/$PERFIL"
source "$HERE/lib_privado.sh"
TMP=$(mktemp -d); PIDS=(); PP=""
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null; done; [ -n "$PP" ] && kill "$PP" 2>/dev/null; rm -rf "$TMP"; privado_parar; }
trap cleanup EXIT
privado_subir || exit 1; privado_dbus || exit 2; privado_tela || exit 2
mkdir -p "$XDG_CONFIG_HOME/iara"
printf 'schema_version = 1\nautostart = true\nshare_output_device = false\nshare_microphone_device = false\ncapture_default_output = false\n' > "$XDG_CONFIG_HOME/iara/config.toml"
cpu_ticks() { awk '{print $14+$15}' "/proc/$1/stat" 2>/dev/null || echo 0; }   # utime+stime em ticks
HZ=$(getconf CLK_TCK)
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/half.wav" synth 120 sine 440 vol 0.5

"$BIN/iara-service" > "$TMP/svc.log" 2>&1 & SVC=$!; PIDS+=($SVC)
for _ in $(seq 1 100); do busctl --user status dev.iara.Mixer >/dev/null 2>&1 && break; sleep 0.1; done
sleep 5
"$BIN/iara-ui" > "$TMP/ui.log" 2>&1 & UI=$!; PIDS+=($UI)
sleep 6
WID=$(xdotool search --onlyvisible --pid "$UI" --name "^Iara$" | head -1)
PWD_PID=$PW_PID
measure() { # título segundos
  local t="$1" secs="$2" a1 a2 a3 b1 b2 b3 n0 n1
  a1=$(cpu_ticks $SVC); a2=$(cpu_ticks $UI); a3=$(cpu_ticks $PWD_PID)
  busctl --user monitor --match "type='signal',interface='dev.iara.Mixer1',member='Levels'" > "$TMP/mon.txt" 2>/dev/null & local M=$!
  sleep "$secs"
  kill $M 2>/dev/null; wait $M 2>/dev/null
  b1=$(cpu_ticks $SVC); b2=$(cpu_ticks $UI); b3=$(cpu_ticks $PWD_PID)
  n1=$(grep -c "Member=Levels\|member=Levels\|‣ Type=signal" "$TMP/mon.txt" || true)
  printf '%-44s serviço %5.2f%%  janela %5.2f%%  pipewire %5.2f%%  Levels/s %5.1f  taps %s\n' "$t" \
    "$(awk -v a=$a1 -v b=$b1 -v h=$HZ -v s=$secs 'BEGIN{print (b-a)/h/s*100}')" \
    "$(awk -v a=$a2 -v b=$b2 -v h=$HZ -v s=$secs 'BEGIN{print (b-a)/h/s*100}')" \
    "$(awk -v a=$a3 -v b=$b3 -v h=$HZ -v s=$secs 'BEGIN{print (b-a)/h/s*100}')" \
    "$(awk -v n=$n1 -v s=$secs 'BEGIN{print n/s}')" "$(pw-cli ls Node | grep -c 'node.name = "iara\.meter\.')"
}
xdotool windowminimize "$WID"; sleep 3
measure "(a) minimizada, silêncio (sem taps)" 15
xdotool windowmap "$WID"; xdotool windowactivate "$WID" 2>/dev/null; sleep 3
measure "(b) medidores ligados, silêncio" 15
pw-play --target iara.ch.game "$TMP/half.wav" & PP=$!; sleep 3
measure "(c) medidores ligados, com som" 15
xdotool windowminimize "$WID"; sleep 3
measure "(d) minimizada, com som (sem taps)" 15
kill "$PP" 2>/dev/null; PP=""
echo "xruns do PipeWire privado: $(grep -ci xrun "$PRIV/pipewire.log" || true)"

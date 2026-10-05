#!/usr/bin/env bash
# Prova 6.3.2 passo 8 (recursos): CPU e RAM do pipewire, do wireplumber e do processo do serviço (sonda) com a topologia
# completa da prova 02 (11 loopbacks no próprio processo), ociosa e com 3 tons; xruns (coluna ERR do pw-top).
# Observação: a sessão do usuário (Zen, Cider, etc.) segue ativa e contribui para o ruído de base.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"
(cd "$HERE/../pw-probe" && cargo build -q)
BIN="$HERE/../pw-probe/target/debug/iara-pw-probe"
TMP=$(mktemp -d); PIDS=(); PROBE=""
cleanup() { for p in "${PIDS[@]:-}" ${PROBE:+$PROBE}; do kill "$p" 2>/dev/null; done; rm -rf "$TMP"; }
trap cleanup EXIT
PW=$(pgrep -x pipewire | head -1); WP=$(pgrep -x wireplumber | head -1)
ticks() { awk '{print $14+$15}' /proc/$1/stat; }
rss()   { awk '/VmRSS/{printf "%.1f", $2/1024}' /proc/$1/status; }
# cpu_window SEGUNDOS pid... -> % de um núcleo por pid
cpu_window() {
  local sec=$1; shift; local a=() b=() i
  for p in "$@"; do a+=("$(ticks "$p")"); done; sleep "$sec"
  for p in "$@"; do b+=("$(ticks "$p")"); done
  local out=""; i=0
  for p in "$@"; do out+=" $(awk -v x="${a[$i]}" -v y="${b[$i]}" -v s="$sec" 'BEGIN{printf "%.1f", (y-x)/100/s*100}')"; i=$((i+1)); done
  echo "$out"
}
xruns() { # ERR (9ª coluna) cumulativo por nó: soma dos nós do Iara e do driver físico principal
  pw-top -b -n 2 2>/dev/null | awk '/^S +ID/{t++} t==2' | awk -v drv="alsa_output.pci-0000_0b_00.4.analog-stereo" '
    $1 ~ /^[RSI]$/ && $2 ~ /^[0-9]+$/ { n=$NF; e=$9; if (n ~ /iara_probe/) {s+=e; c++} else if (n==drv) d=e }
    END { printf "nós_iara=%d soma_ERR_iara=%d ERR_driver_principal=%s", c, s, d }'; echo; }

echo "# pipewire=$PW wireplumber=$WP"
echo "[base, sem topologia, 15 s] CPU% pipewire wireplumber:$(cpu_window 15 $PW $WP)   RSS MiB pw=$(rss $PW) wp=$(rss $WP)"

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"; cmd() { echo "$*" >&9; }
for n in gameA gameB unassigned mixP mixT masterP masterT mic micCommon micApps; do cmd "null $n"; done
cmd "loop A_p gameA mixP"; cmd "loop A_t gameA mixT"; cmd "loop B_p gameB mixP"; cmd "loop B_t gameB mixT"; cmd "loop U_p unassigned mixP"
cmd "loop M_p mixP masterP"; cmd "loop M_t mixT masterT"
cmd "loop mic_in mic micCommon"; cmd "loop mic_apps micCommon micApps"; cmd "loop mic_p micCommon mixP"; cmd "loop mic_t micCommon mixT"
sleep 5
echo "[nós do Iara no grafo: $(pw-cli ls Node | grep -c iara_probe)]"
echo "[topologia ociosa, 20 s] CPU% pipewire wireplumber sonda:$(cpu_window 20 $PW $WP $PROBE)   RSS MiB pw=$(rss $PW) wp=$(rss $WP) sonda=$(rss $PROBE)"

for f in 440 660 880; do sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t$f.wav" synth 40 sine $f vol 0.03; done
pw-play --target iara_probe_gameA "$TMP/t440.wav" & PIDS+=($!)
pw-play --target iara_probe_gameB "$TMP/t660.wav" & PIDS+=($!)
pw-play --target iara_probe_mic   "$TMP/t880.wav" & PIDS+=($!)
sleep 3
echo "[3 tons ativos, 20 s]  CPU% pipewire wireplumber sonda:$(cpu_window 20 $PW $WP $PROBE)   RSS MiB pw=$(rss $PW) wp=$(rss $WP) sonda=$(rss $PROBE)"
echo "[xruns (ERR) por nó durante o teste] $(xruns)"

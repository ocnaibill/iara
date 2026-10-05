#!/usr/bin/env bash
# Prova 6.3.2 (passos 2, 3 e 6 parciais): nós hospedados por um processo Rust (pipewire-rs, tools/pw-probe),
# controlados por Props (channelVolumes/mute) e removidos ao encerrar o processo (SIGKILL).
set -euo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cd "$(dirname "$0")/../pw-probe" && cargo build -q && cd - >/dev/null
BIN="$(dirname "$0")/../pw-probe/target/debug/iara-pw-probe"
TMP=$(mktemp -d); PROBE=""
cleanup() { [ -n "$PROBE" ] && kill "$PROBE" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"
cmd() { echo "$*" >&9; }
count() { pw-cli ls Node | grep -c 'iara_probe_' || true; }

cmd "null chan"; cmd "null pers"; cmd "null tx"
cmd "loop lp chan pers"; cmd "loop lt chan tx"
sleep 3; cmd list; sleep 0.5; cat "$TMP/out"; cat "$TMP/err"
echo "nós no grafo: $(count)"

sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/tone.wav" synth 8 sine 440 vol 0.5
db() { awk -v x="$1" 'BEGIN{ if (x<=1e-7) print "-inf"; else printf "%.2f", 20*log(x)/log(10) }'; }
measure() {
  pw-record -P "{ stream.capture.sink=true }" --target iara_probe_pers --rate 48000 --channels 2 "$TMP/p.wav" & local r1=$!
  pw-record -P "{ stream.capture.sink=true }" --target iara_probe_tx   --rate 48000 --channels 2 "$TMP/t.wav" & local r2=$!
  sleep 1; pw-play --target iara_probe_chan "$TMP/tone.wav" & local pp=$!
  sleep 4; kill $pp 2>/dev/null || true; sleep 0.3
  kill -INT $r1 $r2 2>/dev/null || true; wait $r1 $r2 2>/dev/null || true
  local a b
  a=$(sox "$TMP/p.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}')
  b=$(sox "$TMP/t.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}')
  printf '%-30s pers=%s dB  tx=%s dB\n' "$1" "$(db "$a")" "$(db "$b")"
}
measure "base"
cmd "vol lp_out 0.501"; sleep 0.5; measure "pessoal -6 dB (Props)"
cmd "vol lp_out 1.0";   cmd "mute lt_out true"; sleep 0.5; measure "transmissão mute (flag)"
cmd "mute lt_out false"; sleep 1

echo "--- encerramento abrupto (SIGKILL) ---"
kill -9 "$PROBE"; wait "$PROBE" 2>/dev/null || true; PROBE=""
sleep 2
echo "nós restantes após SIGKILL: $(count)"

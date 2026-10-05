#!/usr/bin/env bash
# Prova 6.3.2 passo 8 (latência): atraso de um clique através de 1, 2 e 3 loopbacks em série, com os nós hospedados
# pelo processo Rust. Dois taps simétricos (pw-loopback) copiam a origem para FL e o fim do caminho para FR de um sink
# estéreo; o atraso do caminho é a diferença entre os canais (o atraso do tap cancela).
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"
(cd "$HERE/../pw-probe" && cargo build -q)
BIN="$HERE/../pw-probe/target/debug/iara-pw-probe"
TMP=$(mktemp -d); PIDS=(); PROBE=""
cleanup() { for p in "${PIDS[@]:-}" ${PROBE:+$PROBE}; do kill "$p" 2>/dev/null; done; rm -rf "$TMP"; }
trap cleanup EXIT
echo "# $(pw-metadata -n settings 2>/dev/null | grep -E "clock\.(rate|quantum)'" | sed -E "s/.*key:'([^']*)' value:'([^']*)'.*/\1=\2/" | tr '\n' ' ')"

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"; cmd() { echo "$*" >&9; }
for n in chan mic micCommon micApps mixP mixT masterP masterT meas ctl; do cmd "null $n"; done
cmd "loop A chan mixP";        cmd "loop B mixP masterP"
cmd "loop C mic micCommon";    cmd "loop D micCommon micApps"
cmd "loop E micCommon mixT";   cmd "loop F mixT masterT"
# controle positivo: loopback com atraso conhecido de 50 ms (a ferramenta tem de enxergá-lo)
pw-loopback -n iara_probe_ctl_lp -d 0.05 \
  --capture-props="{ node.name=iara_probe_ctl_lp_in target.object=iara_probe_chan stream.capture.sink=true node.passive=true state.restore-props=false state.restore-target=false }" \
  --playback-props="{ node.name=iara_probe_ctl_lp_out target.object=iara_probe_ctl node.dont-fallback=true state.restore-props=false state.restore-target=false }" &
PIDS+=($!)
sleep 4

python3 - "$TMP/click.wav" <<'PY'
import wave, struct, sys
w = wave.open(sys.argv[1], "wb"); w.setnchannels(2); w.setsampwidth(2); w.setframerate(48000)
fr = bytearray()
for i in range(48000):
    v = 20000 if 24000 <= i < 24010 else 0
    fr += struct.pack("<hh", v, v)
w.writeframes(bytes(fr)); w.close()
PY

tap() { # $1 sink monitor a copiar, $2 canal de destino (FL|FR), $3 nome
  pw-loopback -n "iara_probe_tap$3" -c 1 \
    --capture-props="{ node.name=iara_probe_tap$3_in target.object=iara_probe_$1 stream.capture.sink=true node.passive=true audio.position=[FL] state.restore-props=false state.restore-target=false }" \
    --playback-props="{ node.name=iara_probe_tap$3_out target.object=iara_probe_meas audio.position=[$2] node.dont-fallback=true state.restore-props=false state.restore-target=false }" &
  PIDS+=($!)
}
measure() { # $1 origem  $2 fim ; imprime atraso em ms
  tap "$1" FL 1; local t1=$!; tap "$2" FR 2; local t2=$!
  sleep 2
  pw-record -P "{ stream.capture.sink=true }" --target iara_probe_meas --rate 48000 --channels 2 "$TMP/m.wav" & local r=$!
  sleep 0.7; pw-play --target "iara_probe_$1" "$TMP/click.wav"; sleep 0.5
  kill -INT $r 2>/dev/null; wait $r 2>/dev/null
  kill $t1 $t2 2>/dev/null; wait $t1 $t2 2>/dev/null; sleep 0.5
  sox "$TMP/m.wav" -t raw -e signed-integer -b 16 -c 2 -r 48000 "$TMP/m.raw" 2>/dev/null
  python3 - "$TMP/m.raw" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read(); n = len(d) // 4
s = struct.unpack("<%dh" % (2 * n), d[: 4 * n])
L, R = s[0::2], s[1::2]
def first(x):
    m = max(abs(v) for v in x)
    if m < 500: return None
    return next(i for i, v in enumerate(x) if abs(v) >= m * 0.5)
a, b = first(L), first(R)
print("sem sinal" if a is None or b is None else f"{(b - a) / 48:.1f}")
PY
}
run() { # $1 título $2 origem $3 fim
  local out=() v
  for i in 1 2 3 4 5; do v=$(measure "$2" "$3"); out+=("$v"); done
  printf '%-34s ms: %s\n' "$1" "${out[*]}"
}
run "CONTROLE: loopback com -d 0.05 ms"      chan   ctl
run "chan → mixP (1 loopback)"           chan   mixP
run "chan → masterP (2 em série)"        chan   masterP
run "mic → micApps (2 em série)"         mic    micApps
run "mic → masterT (3 em série)"         mic    masterT

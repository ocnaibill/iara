#!/usr/bin/env bash
# Ponta a ponta do motor real (iara-audio): perfil → plano → PipeWire, medido nos MASTERs e na fonte do MIC.
# Cenários da spec 12. O processo é o exemplo mixer_cli; nós `iara.*` no grafo real da sessão enquanto roda.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-audio --examples) || exit 1
BIN="$ROOT/target/debug/examples/mixer_cli"
TMP=$(mktemp -d); PROC=""
cleanup() { [ -n "$PROC" ] && kill "$PROC" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
count() { pw-cli ls Node | grep -c 'node.name = "iara\.' || true; }

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROC=$!
exec 9> "$TMP/in"; cmd() { echo "$*" >&9; sleep 1.5; }
sleep 6; echo "aplicação inicial: $(head -1 "$TMP/out")"; echo "nós iara.* no grafo: $(count)"
echo "# como as fontes aparecem para outros aplicativos (pw-dump):"
pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if o['type']=='PipeWire:Interface:Node' and p.get('media.class')=='Audio/Source' and p.get('node.name','').startswith('iara.'):
        print('  ', p['node.name'], '|', p['media.class'], '|', p.get('node.description'))
"

sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 8 sine 440 vol 0.5
db() { awk -v x="$1" 'BEGIN{ if (x<=1e-7) print "-inf"; else printf "%.2f", 20*log(x)/log(10) }'; }
run() { # título origem alvo...
  local title="$1" src="$2"; shift 2; local rec=() t
  for t in "$@"; do
    # fontes virtuais são lidas como um cliente de captura (OBS/Discord) lê; os demais nós, pelo monitor
    if [[ "$t" == iara.src.* ]]; then pw-record --target "$t" --rate 48000 --channels 2 "$TMP/r_$t.wav" & rec+=($!)
    else pw-record -P "{ stream.capture.sink=true }" --target "$t" --rate 48000 --channels 2 "$TMP/r_$t.wav" & rec+=($!); fi
  done
  sleep 1; pw-play --target "$src" "$TMP/t.wav" & local pp=$!
  sleep 3.5; kill $pp 2>/dev/null; sleep 0.3; kill -INT "${rec[@]}" 2>/dev/null; wait "${rec[@]}" 2>/dev/null
  local out="" v
  for t in "$@"; do v=$(sox "$TMP/r_$t.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}'); out+=" ${t#iara.}=$(db "$v")"; done
  printf '%-46s%s\n' "$title" "$out"
}
P=iara.master.personal; T=iara.master.transmission; A=iara.mic.apps; SM=iara.src.mic; ST=iara.src.transmission
echo "# dBFS RMS (seno 440 Hz, amplitude 0,5)"
run "GAME (base): fonte Transmissão = MASTER transm."  iara.ch.game $P $T $ST $SM
cmd "gain game personal -6";   run "GAME: escuta -6 dB"                    iara.ch.game $P $T; cmd "gain game personal 0"
cmd "enable media transmission false"; run "MEDIA: fora da transmissão"    iara.ch.media $P $T; cmd "enable media transmission true"
run "AUX (padrão: fora da transmissão)"    iara.ch.aux $P $T
run "Não atribuídos (só escuta)"           iara.unassigned $P $T
cmd "master-gain transmission -6"; run "MASTER transmissão -6 dB (GAME)"  iara.ch.game $P $T $ST $SM; cmd "master-gain transmission 0"
cmd "chatmix 1";  run "ChatMix +1: GAME"   iara.ch.game $P $T; run "ChatMix +1: CHAT" iara.ch.chat $P $T; cmd "chatmix 0"
run "MIC (base)" iara.mic.input $P $T $A $SM $ST
cmd "mic-mute true";  run "MIC mute global"  iara.mic.input $P $T $A $SM $ST; cmd "mic-mute false"
cmd "mic-gain -20";   run "MIC ganho -20 dB" iara.mic.input $P $T $A; cmd "mic-gain 0"
cmd "master-gain personal -20"; run "MASTER pessoal -20: MIC apps intocado" iara.mic.input $P $T $A $SM $ST; cmd "master-gain personal 0"
echo "reaplicações: $(grep -c '^ok' "$TMP/out") ok, $(grep -c -v '^ok' "$TMP/out") outras; nós iara.* ao fim: $(count)"
echo quit >&9; wait "$PROC" 2>/dev/null; PROC=""; sleep 1
echo "nós iara.* após encerrar o motor: $(count)"

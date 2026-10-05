#!/usr/bin/env bash
# Prova 6.3.2 passos 5-6 (parcial): ausência e retorno de dispositivo físico (perfil de placa USB desligado/religado por software).
# Compara loopbacks com node.dont-fallback (nofb) e sem (fb), para entrada (microfone) e saída (fone), usando o fifine AM8 Pro.
# ATENÇÃO: o fifine fica ausente por alguns segundos; a fonte padrão do sistema pode mudar e voltar.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"
(cd "$HERE/../pw-probe" && cargo build -q)
BIN="$HERE/../pw-probe/target/debug/iara-pw-probe"
TMP=$(mktemp -d); PROBE=""; DEV=""; ORIG=""
cleanup() {
  [ -n "$DEV" ] && [ -n "$ORIG" ] && pw-cli s "$DEV" Profile "{ index: $ORIG, save: false }" >/dev/null 2>&1
  [ -n "$PROBE" ] && kill "$PROBE" 2>/dev/null
  rm -rf "$TMP"
}
trap cleanup EXIT

read -r DEV ORIG NAME < <(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    if o['type']=='PipeWire:Interface:Device' and 'fifine' in o['info']['props'].get('device.name',''):
        pr=o['info']['params']['Profile'][0]; print(o['id'], pr['index'], pr['name'])")
echo "fifine: device $DEV, perfil original índice $ORIG ($NAME)"
SRC=alsa_input.usb-MV-SILICON_fifine_AM8_Pro_20190808-00.mono-fallback
SNK=alsa_output.usb-MV-SILICON_fifine_AM8_Pro_20190808-00.analog-stereo
ALL="$SRC $SNK iara_probe_mA_in iara_probe_mB_in iara_probe_oA_out iara_probe_oB_out"
defaults() { echo "   padrões: sink=$(pw-metadata -n default 0 default.audio.sink 2>/dev/null | grep -o 'name":"[^"]*' | head -1 | cut -d'"' -f3)  fonte=$(pw-metadata -n default 0 default.audio.source 2>/dev/null | grep -o 'name":"[^"]*' | head -1 | cut -d'"' -f3)"; }
report() { echo "[$1]"; python3 "$HERE/lib_links.py" $ALL; defaults; }

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"; cmd() { echo "$*" >&9; }
cmd "null chan"; cmd "null micCommon"
cmd "lpx mA $SRC 0 iara_probe_micCommon nofb"; cmd "lpx mB $SRC 0 iara_probe_micCommon fb"
cmd "lpx oA iara_probe_chan 1 $SNK nofb";       cmd "lpx oB iara_probe_chan 1 $SNK fb"
sleep 4; report "vivo"

pw-cli s "$DEV" Profile "{ index: 0, save: false }" >/dev/null; sleep 4; report "dispositivo ausente (perfil off, 4 s)"
pw-cli s "$DEV" Profile "{ index: $ORIG, save: false }" >/dev/null; sleep 6; report "dispositivo de volta (6 s)"
cat "$TMP/err" | head -5

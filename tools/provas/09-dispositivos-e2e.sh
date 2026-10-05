#!/usr/bin/env bash
# Ciclo de vida dos dispositivos físicos no motor real (spec 6.3.1.3, 8.5, 8.6): ligar, ausência sem fallback,
# retorno automático e “escolha durante a ausência prevalece”. Usa HDMI (saída) e a placa de captura ezcap
# (entrada) por não terem outros usuários; os perfis das placas são desligados/religados por software e restaurados.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-audio --examples) || exit 1
BIN="$ROOT/target/debug/examples/mixer_cli"
OUT=alsa_output.pci-0000_09_00.1.hdmi-stereo
IN=alsa_input.usb-ezcap_ezcap_LIVE_GAMER_RAW_00000001-02.analog-stereo
TMP=$(mktemp -d); PROC=""; RESTORE=()
cleanup() { for r in "${RESTORE[@]:-}"; do pw-cli s ${r%%:*} Profile "{ index: ${r##*:}, save: false }" >/dev/null 2>&1; done
  [ -n "$PROC" ] && kill "$PROC" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
dev_of() { pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    if o['type']=='PipeWire:Interface:Device' and '$1' in o['info']['props'].get('device.name',''):
        print(o['id'], o['info']['params']['Profile'][0]['index'])"; }
read -r HDMI_DEV HDMI_IDX < <(dev_of pci-0000_09_00.1); read -r EZ_DEV EZ_IDX < <(dev_of ezcap)
RESTORE=("$HDMI_DEV:$HDMI_IDX" "$EZ_DEV:$EZ_IDX")
echo "# placas: HDMI dev=$HDMI_DEV (perfil $HDMI_IDX), ezcap dev=$EZ_DEV (perfil $EZ_IDX)"

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROC=$!
exec 9> "$TMP/in"; cmd() { echo "$1" >&9; sleep "${2:-1.5}"; }
events() { : > "$TMP/ev"; echo events >&9; sleep 0.7; awk '/^evento/' "$TMP/out" | tail -n +"$((EVSEEN+1))" ; EVSEEN=$(awk '/^evento/' "$TMP/out" | wc -l); }
EVSEEN=0
state() { echo "   [$1]"; python3 "$HERE/lib_links.py" iara.dev.output.out iara.dev.input.in "$OUT" "$IN" | sed 's/^/   /'; }
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 8 sine 440 vol 0.03
db() { awk -v x="$1" 'BEGIN{ if (x<=1e-7) print "-inf"; else printf "%.2f", 20*log(x)/log(10) }'; }
tone_at_hdmi() { # mede o tom (via iara.ch.game → MASTER pessoal → ligação → monitor do HDMI)
  pw-record -P "{ stream.capture.sink=true }" --target "$OUT" --rate 48000 --channels 2 "$TMP/h.wav" & local r=$!
  sleep 1; pw-play --target iara.ch.game "$TMP/t.wav" & local p=$!; sleep 3.5; kill $p 2>/dev/null; sleep 0.3
  kill -INT $r 2>/dev/null; wait $r 2>/dev/null
  local v; v=$(sox "$TMP/h.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}'); echo "   tom no monitor do HDMI: $(db "$v") dBFS (esperado ≈ −33,5 com o tom de amplitude 0,03 se houver ligação)"; }

sleep 6
echo "== 1. preferências definidas"
cmd "output $OUT" 4; cmd "mic-device $IN" 4
tail -2 "$TMP/out" | sed 's/^/   /'; state "ligado"; tone_at_hdmi; events

echo "== 2. HDMI some (perfil off)"
pw-cli s "$HDMI_DEV" Profile "{ index: 0, save: false }" >/dev/null; sleep 4
state "HDMI ausente"; events
echo "   outros nós sinks recebendo de iara.*: $(python3 "$HERE/lib_links.py" iara.master.personal | sed 's/^ *//')"

echo "== 3. HDMI volta"
pw-cli s "$HDMI_DEV" Profile "{ index: $HDMI_IDX, save: false }" >/dev/null; sleep 7
state "HDMI de volta (7 s)"; tone_at_hdmi; events

echo "== 4. ezcap some e volta"
pw-cli s "$EZ_DEV" Profile "{ index: 0, save: false }" >/dev/null; sleep 4; state "ezcap ausente"; events
pw-cli s "$EZ_DEV" Profile "{ index: $EZ_IDX, save: false }" >/dev/null; sleep 7; state "ezcap de volta (7 s)"; events

echo "== 5. usuário escolhe outra saída durante a ausência (nenhuma): a volta não recria a ligação antiga"
pw-cli s "$HDMI_DEV" Profile "{ index: 0, save: false }" >/dev/null; sleep 4
cmd "output none" 2; pw-cli s "$HDMI_DEV" Profile "{ index: $HDMI_IDX, save: false }" >/dev/null; sleep 7
state "HDMI voltou, preferência = nenhuma"; events

echo quit >&9; wait "$PROC" 2>/dev/null; PROC=""; sleep 1
echo "nós iara.* após encerrar: $(pw-cli ls Node | grep -c 'node.name = "iara\.')"

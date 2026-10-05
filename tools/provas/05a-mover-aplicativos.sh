#!/usr/bin/env bash
# Prova 6.3.2 passo 7: mover aplicativos por metadata, node.dont-move, destino explícito e mudança externa observável.
# Streams de teste (pw-play, tom baixo): A normal, B com node.dont-move=true, C com destino explícito na saída física.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"
(cd "$HERE/../pw-probe" && cargo build -q)
BIN="$HERE/../pw-probe/target/debug/iara-pw-probe"
PHYS=alsa_output.pci-0000_0b_00.4.analog-stereo
TMP=$(mktemp -d); PIDS=(); PROBE=""
cleanup() { for p in "${PIDS[@]:-}" ${PROBE:+$PROBE}; do kill "$p" 2>/dev/null; done; rm -rf "$TMP"; }
trap cleanup EXIT
mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"; echo "null chan" >&9; sleep 2
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/tone.wav" synth 90 sine 440 vol 0.03
pw-play -P media.name=A "$TMP/tone.wav" & PIDS+=($!)
pw-play -P media.name=B -P node.dont-move=true "$TMP/tone.wav" & PIDS+=($!)
pw-play -P media.name=C --target "$PHYS" "$TMP/tone.wav" & PIDS+=($!)
sleep 2
nid() { pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('media.name')=='$1' and o['type']=='PipeWire:Interface:Node': print(o['id'])"; }
A=$(nid A); B=$(nid B); C=$(nid C)
where() { python3 - <<'PY'
import json,subprocess
d=json.loads(subprocess.run(["pw-dump"],capture_output=True,text=True).stdout)
nodes={o["id"]:o["info"]["props"] for o in d if o["type"]=="PipeWire:Interface:Node"}
links=[o["info"] for o in d if o["type"]=="PipeWire:Interface:Link"]
for i,p in sorted(nodes.items(), key=lambda kv: kv[1].get("media.name","")):
    if p.get("media.name") in ("A","B","C"):
        dst=sorted({nodes.get(l["input-node-id"],{}).get("node.name","?") for l in links if l["output-node-id"]==i})
        dm=p.get("node.dont-move","—")
        short=[x.replace("alsa_output.pci-0000_0b_00.4.analog-stereo","FÍSICA") for x in dst]
        print(f"   {p['media.name']} dont-move={dm:<5} -> {short or 'SEM LINK'}")
PY
}
echo "[1] inicial"; where
for s in $A $B $C; do pw-metadata -n default "$s" target.object iara_probe_chan Spa:String >/dev/null; done
sleep 2; echo "[2] após metadata target.object=iara_probe_chan nos três"; where

pw-metadata -n default -m > "$TMP/mon" 2>&1 & MON=$!; PIDS+=($MON); sleep 1
pw-metadata -n default "$A" target.object "$PHYS" Spa:String >/dev/null   # "ferramenta externa" move A de volta
sleep 2; echo "[3] mudança externa em A (outra ferramenta, via metadata) para a saída física"; where
echo "    metadata visto por um observador independente (id de A = $A):"; grep -E "id:$A key:" "$TMP/mon" | tail -2 | cut -c1-120 | sed 's/^/      /'

# ferramenta externa real: pactl (protocolo Pulse, usado pelo Plasma/pavucontrol) move A para o canal do Iara
SI=$(pactl list sink-inputs | awk '/Sink Input #/{id=substr($3,2)} /media.name = "A"/{print id}')
pactl move-sink-input "$SI" iara_probe_chan; sleep 2
echo "[4] pactl move-sink-input (ferramenta externa) de A para iara_probe_chan"; where
echo "    metadata observado:"; grep -E "id:$A key:" "$TMP/mon" | tail -2 | cut -c1-120 | sed 's/^/      /'
# limpeza: só as chaves dos streams de teste (nunca apagar chaves de outros aplicativos)
for s in $A $B $C; do pw-metadata -n default -d "$s" target.object >/dev/null 2>&1; pw-metadata -n default -d "$s" target.node >/dev/null 2>&1; done

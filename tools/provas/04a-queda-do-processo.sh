#!/usr/bin/env bash
# Prova 6.3.2 passo 6 (parcial): queda do processo proprietário dos nós (SIGKILL) com streams de teste ativos.
# S1: pw-play com destino explícito num sink do Iara; S2: pw-play sem destino (controle, segue o padrão).
# Tom baixo (amplitude 0,05): em caso de fallback ele toca na saída padrão por alguns segundos.
set -euo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"
(cd "$HERE/../pw-probe" && cargo build -q)
BIN="$HERE/../pw-probe/target/debug/iara-pw-probe"
TMP=$(mktemp -d); PIDS=(); PROBE=""
cleanup() { for p in "${PIDS[@]:-}" ${PROBE:+$PROBE}; do kill "$p" 2>/dev/null || true; done; rm -rf "$TMP"; }
trap cleanup EXIT

# Para cada stream pw-play: nome do nó de destino ligado a ele (via pw-dump).
where() {
python3 - "$1" <<'PY'
import json,subprocess,sys
d=json.loads(subprocess.run(["pw-dump"],capture_output=True,text=True).stdout)
nodes={o["id"]:o["info"]["props"] for o in d if o["type"]=="PipeWire:Interface:Node"}
links=[o["info"] for o in d if o["type"]=="PipeWire:Interface:Link"]
for nid,p in sorted(nodes.items()):
    if p.get("application.name")=="pw-cat" or p.get("node.name","").startswith("pw-play"):
        if p.get("media.name","")!=sys.argv[1] and sys.argv[1]!="*": continue
        dst=sorted({nodes.get(l["input-node-id"],{}).get("node.name","?") for l in links if l["output-node-id"]==nid})
        print(f"  stream {p.get('media.name','?'):>6} (id {nid}) -> {dst or 'SEM LINK'}")
PY
}

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"; cmd() { echo "$*" >&9; }
cmd "null chan"; cmd "null pers"; cmd "null tx"; cmd "loop lp chan pers"; cmd "loop lt chan tx"
sleep 3
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/tone.wav" synth 60 sine 440 vol 0.05
pw-play -P media.name=S1 --target iara_probe_chan "$TMP/tone.wav" & PIDS+=($!)
pw-play -P media.name=S2 "$TMP/tone.wav" & PIDS+=($!)
sleep 2
echo "[1] com o processo vivo:"; where '*'

kill -9 "$PROBE"; wait "$PROBE" 2>/dev/null || true; PROBE=""; sleep 3
echo "[2] 3 s após SIGKILL do processo:"; where '*'
echo "    nós iara_probe restantes: $(pw-cli ls Node | grep -c iara_probe || true)"

mkfifo "$TMP/in2"; "$BIN" < "$TMP/in2" > "$TMP/out2" 2> "$TMP/err2" & PROBE=$!
exec 8> "$TMP/in2"; c2() { echo "$*" >&8; }
c2 "null chan"; c2 "null pers"; c2 "null tx"; c2 "loop lp chan pers"; c2 "loop lt chan tx"
sleep 4
echo "[3] 4 s após recriar os mesmos nós:"; where '*'

S1=$(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('media.name')=='S1' and o['type']=='PipeWire:Interface:Node': print(o['id'])")
pw-metadata -n default "$S1" target.object iara_probe_chan Spa:String >/dev/null
sleep 2
echo "[4] 2 s após metadata target.object=iara_probe_chan no stream S1 (id $S1):"; where '*'
pw-metadata -n default "$S1" target.object >/dev/null 2>&1 || pw-metadata -n default -d "$S1" target.object >/dev/null 2>&1 || true

#!/usr/bin/env bash
# Aplicativos de ponta a ponta: serviço + motor + D-Bus + janela, com fluxos de teste (pw-play) IaraTeste{A,B,C}.
# Associa por regra (AssignApp), muda só nesta sessão (SetAppSession), remove canal com destino das regras, e confere
# os links REAIS no PipeWire. Só os fluxos de teste são movidos; os aplicativos do usuário aparecem como Não atribuídos.
# A janela aparece por alguns segundos. Saída: PNGs em $1 (padrão ./capturas).
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
OUT="${1:-$ROOT/capturas}"; mkdir -p "$OUT"
(cd "$ROOT" && cargo build -q -p iara-service -p iara-ui --features gtk-ui) || exit 1
TMP=$(mktemp -d); SVC=""; UI=""; PIDS=()
cleanup() { for p in "${PIDS[@]:-}" ${UI:+$UI} ${SVC:+$SVC}; do kill "$p" 2>/dev/null; done
  for id in $(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('application.name','').startswith('IaraTeste'): print(o['id'])" 2>/dev/null); do pw-metadata -n default -d "$id" target.object >/dev/null 2>&1; done
  rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state"
N="dev.iara.MixerApps$$"; export IARA_BUS_NAME="$N"; O=/dev/iara/Mixer; I=dev.iara.Mixer1
bc() { busctl --user call -- "$N" "$O" "$I" "$@" >/dev/null; }
where() { pw-dump | python3 -c "
import json,sys
d=json.load(sys.stdin)
nodes={o['id']:o['info']['props'] for o in d if o['type']=='PipeWire:Interface:Node'}
links=[o['info'] for o in d if o['type']=='PipeWire:Interface:Link']
for i,p in sorted(nodes.items(), key=lambda kv: kv[1].get('application.name','')):
    if p.get('application.name','').startswith('IaraTeste'):
        dst=sorted({nodes.get(l['input-node-id'],{}).get('node.name','?').replace('alsa_output.pci-0000_0b_00.4.analog-stereo','FÍSICA') for l in links if l['output-node-id']==i})
        print('   ', p['application.name'], '->', dst or 'sem link')"; }

"$ROOT/target/debug/iara-service" > "$TMP/svc.log" 2>&1 & SVC=$!
for i in $(seq 1 100); do busctl --user status "$N" >/dev/null 2>&1 && break; sleep 0.1; done
sleep 1
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 120 sine 440 vol 0.02
play() { pw-play -P "{ application.name=IaraTeste$1 application.process.binary=iarateste$2 media.name=fluxo$1 ${3:-} }" "$TMP/t.wav" & PIDS+=($!); }
play A a; play B b; play C c "node.dont-move=true"
sleep 2

IARA_UI_SHOT_DELAY_MS=14000 "$ROOT/target/debug/iara-ui" --screenshot "$OUT/ui-aplicativos.png" 2>&1 | grep -E "captura|falha" & UI=$!
sleep 2.5
echo "1. sem regras: os fluxos de teste seguem na saída física (Não atribuídos)"; where
echo "2. regras salvas (AssignApp): A → game, B → media, C → chat (C não aceita ser movido)"
bc AssignApp ssss "" iaratestea "" game; bc AssignApp ssss "" iaratesteb "" media; bc AssignApp ssss "" iaratestec "" chat
sleep 4; where
echo "3. só nesta sessão (SetAppSession): A → chat; o perfil NÃO muda"
bc SetAppSession ss bin:iaratestea chat; sleep 3; where
echo "   regra salva de A no perfil: $(grep -B1 -A3 'iaratestea' "$TMP/config/iara/profiles/default.toml" | grep channel | tr -d ' ')"
echo "4. remover o canal media com destino 'game': a regra de B passa para game"
bc RemoveChannel ss media game; sleep 4; where
echo "   regras no perfil: $(grep -A3 '^\[\[rules\]\]' "$TMP/config/iara/profiles/default.toml" | grep -E 'binary|channel' | tr -d ' ' | paste -sd' ')"
wait $UI; UI=""
echo "5. fim: encerrar o serviço; fluxos de teste voltam ao padrão?"
kill -TERM "$SVC"; wait "$SVC" 2>/dev/null; SVC=""; sleep 2; where

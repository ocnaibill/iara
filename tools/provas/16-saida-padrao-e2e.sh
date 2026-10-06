#!/usr/bin/env bash
# Saída padrão do sistema (spec 8.2) contra o PipeWire REAL: instalação, aplicativo novo entrando pelo mixer, aplicativo com
# saída própria, queda e recuperação (--restore-default), escolha do usuário e "Desligar mixer".
# ATENÇÃO: muda a saída padrão do sistema durante o teste; seus aplicativos (que seguem o padrão) passam pelo mixer, que sai
# pelo mesmo dispositivo, com pequenos cortes a cada troca. Rede de segurança: o `trap` devolve o padrão original sozinho.
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-service) || exit 1
BIN="$ROOT/target/debug/iara-service"
cfg() { pw-metadata -n default 0 default.configured.audio.sink 2>/dev/null | grep -o 'name":"[^"]*' | head -1 | cut -d'"' -f3; }
ORIG=$(cfg)
[ -n "$ORIG" ] || { echo "sem saída padrão configurada; não vou testar"; exit 1; }
echo "# saída padrão original: $ORIG"
TMP=$(mktemp -d); SVC=""; PIDS=()
restore_original() { [ "$(cfg)" = "iara.unassigned" ] && pw-metadata -n default 0 default.configured.audio.sink "{\"name\":\"$ORIG\"}" Spa:String:JSON >/dev/null 2>&1; }
cleanup() { for p in "${PIDS[@]:-}" ${SVC:+$SVC}; do kill "$p" 2>/dev/null; done; sleep 0.5
  for id in $(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('application.name','').startswith('IaraTeste'): print(o['id'])" 2>/dev/null); do pw-metadata -n default -d "$id" target.object >/dev/null 2>&1; done
  restore_original; sleep 1; echo "# padrão ao sair: $(cfg) (original: $ORIG)"; rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state"
N="dev.iara.MixerDS$$"; export IARA_BUS_NAME="$N"; O=/dev/iara/Mixer; I=dev.iara.Mixer1
bc() { busctl --user call -- "$N" "$O" "$I" "$@" >/dev/null; }
state() { busctl --user --json=short call -- "$N" "$O" "$I" GetState 2>/dev/null | python3 -c "
import json,sys
d=json.load(sys.stdin)['data']
print('default_output=%s conectado=%s ausentes=%s' % (d[7], d[2], d[4]))
import re
apps=d[6]
for m in re.finditer(r'display = \"([^\"]*)\"(?:.*?)\nstate = \"([^\"]*)\"', apps, re.S):
    print('   app', m.group(1), '->', m.group(2))" 2>/dev/null; }
wait_cfg() { for i in $(seq 1 60); do [ "$(cfg)" = "$1" ] && return 0; sleep 0.25; done; return 1; }
links() { # nó -> sinks ligados
  pw-dump | python3 -c "
import json,sys
want=sys.argv[1:]
d=json.load(sys.stdin)
nodes={o['id']:o['info']['props'] for o in d if o['type']=='PipeWire:Interface:Node'}
links=[o['info'] for o in d if o['type']=='PipeWire:Interface:Link']
for i,p in sorted(nodes.items(), key=lambda kv: kv[1].get('application.name','')):
    if p.get('application.name') in want:
        dst=sorted({nodes.get(l['input-node-id'],{}).get('node.name','?') for l in links if l['output-node-id']==i})
        print('   ', p['application.name'], '->', dst)" "$@"; }
start_service() { "$BIN" > "$TMP/svc.log" 2>&1 & SVC=$!; for i in $(seq 1 100); do busctl --user status "$N" >/dev/null 2>&1 && break; sleep 0.1; done; }

echo "1. iniciar o serviço (primeira execução, sem saída preferida)"
start_service
wait_cfg iara.unassigned && echo "   padrão do sistema agora: $(cfg)" || echo "   NÃO instalou (padrão: $(cfg))"
sleep 1
echo "   saída preferida adotada do padrão anterior: $(grep -A0 'output = ' "$TMP/config/iara/profiles/default.toml" | tr -d ' ')"
echo "   registro do anterior: $(cat "$TMP/state/iara/default-sink.toml" 2>/dev/null | tr '\n' ' ')"
echo "   seus aplicativos (seguem o padrão) agora passam pelo Iara:"; links Zen Cider
state

echo "2. aplicativo NOVO sem destino entra pelo mixer; outro com saída própria fica fora"
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 120 sine 440 vol 0.02
pw-play -P "{ application.name=IaraTesteNovo application.process.binary=iaratestenovo media.name=novo }" "$TMP/t.wav" & PIDS+=($!)
pw-play -P "{ application.name=IaraTestePropria application.process.binary=iaratestepropria media.name=propria }" --target "$ORIG" "$TMP/t.wav" & PIDS+=($!)
sleep 3; links IaraTesteNovo IaraTestePropria; state

echo "3. associar o app novo ao canal game (regra salva)"
bc AssignApp ssss "" iaratestenovo "" game; sleep 3; links IaraTesteNovo

echo "4. QUEDA: kill -9 no serviço (o padrão fica no Iara) e recuperação com --restore-default"
kill -9 "$SVC"; wait "$SVC" 2>/dev/null; SVC=""; sleep 1.5
echo "   após a queda: padrão = $(cfg) (esperado iara.unassigned); registro presente: $([ -f "$TMP/state/iara/default-sink.toml" ] && echo sim || echo não)"
"$BIN" --restore-default 2>&1 | sed 's/^/   /'
echo "   após --restore-default: padrão = $(cfg) (esperado $ORIG); registro presente: $([ -f "$TMP/state/iara/default-sink.toml" ] && echo sim || echo não)"
echo "   seus aplicativos de volta:"; links Zen Cider

echo "5. reiniciar o serviço e depois o USUÁRIO escolhe a saída original explicitamente: o Iara respeita"
start_service; wait_cfg iara.unassigned && echo "   instalou de novo"; sleep 1
pw-metadata -n default 0 default.configured.audio.sink "{\"name\":\"$ORIG\"}" Spa:String:JSON >/dev/null
sleep 2; state | head -1
echo "   registro apagado (posse largada): $([ -f "$TMP/state/iara/default-sink.toml" ] && echo NÃO || echo sim)"
echo "6. 'Desligar mixer' com posse largada: não mexe na escolha do usuário"
bc Deactivate; wait "$SVC" 2>/dev/null; SVC=""; sleep 1; echo "   padrão = $(cfg) (esperado $ORIG)"

echo "7. instalar de novo e 'Desligar mixer' de verdade: restaura o anterior"
start_service; wait_cfg iara.unassigned && echo "   instalou: $(cfg)"; sleep 1
bc Deactivate; wait "$SVC" 2>/dev/null; SVC=""; sleep 1.5
echo "   padrão = $(cfg) (esperado $ORIG); nós iara.*: $(pw-cli ls Node | grep -c 'node.name = "iara\.')"
echo "   seus aplicativos:"; links Zen Cider

#!/usr/bin/env bash
# Roteamento de aplicativos no motor real: inventário de fluxos, movimento por metadata, confirmação pelos links reais,
# node.dont-move, "não brigar" com mudança externa e devolução ao padrão. Fluxos de teste (pw-play, tom bem baixo) com
# identidades próprias; o destino padrão do sistema NÃO é alterado e nenhum aplicativo do usuário é tocado.
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-audio --examples) || exit 1
BIN="$ROOT/target/debug/examples/mixer_cli"
TMP=$(mktemp -d); PROC=""; PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null; done; [ -n "$PROC" ] && kill "$PROC" 2>/dev/null
  # remove só as chaves de metadata dos nossos fluxos de teste (nunca as de outros aplicativos)
  for id in $(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('application.name','').startswith('IaraTeste'): print(o['id'])" 2>/dev/null); do pw-metadata -n default -d "$id" target.object >/dev/null 2>&1; done
  rm -rf "$TMP"; }
trap cleanup EXIT
mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROC=$!
exec 9> "$TMP/in"; cmd() { echo "$1" >&9; sleep "${2:-1.2}"; }
sleep 6
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 120 sine 440 vol 0.02
play() { # nome binário [propriedade extra]
  pw-play -P "{ application.name=IaraTeste$1 application.process.binary=iarateste$2 media.name=fluxo$1 ${3:-} }" "$TMP/t.wav" & PIDS+=($!); }
play A a; play B b; play C c "node.dont-move=true"
sleep 2
last_apps() { awk '/^--$/{blk=cur; cur=""; next} /^app IaraTeste/{cur=cur "   " substr($0,5) "\n"} END{printf "%s", blk}' "$TMP/out"; }
ev() { echo events >&9; sleep 0.9; last_apps; }

echo "1. inventário (sem rotas): os três aplicativos aparecem, sem estado de rota"
ev
echo "2. rotas: A → GAME, B → MEDIA, C → GAME (C recusa ser movido)"
cmd "route bin:iaratestea iara.ch.game" 0.3; cmd "route bin:iaratesteb iara.ch.media" 0.3; cmd "route bin:iaratestec iara.ch.game" 2.5
ev
echo "3. A passa de GAME para CHAT (nova decisão do usuário)"
cmd "route bin:iaratestea iara.ch.chat" 2.5; ev
echo "4. alguém move B por fora (pw-metadata) para GAME: o motor NÃO briga"
BID=$(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('application.name')=='IaraTesteB' and o['type']=='PipeWire:Interface:Node': print(o['id'])")
pw-metadata -n default "$BID" target.object iara.ch.game Spa:String >/dev/null
sleep 5; ev
echo "5. A volta a Não atribuídos (padrão do sistema): a sobreposição do Iara é removida"
cmd "route bin:iaratestea default" 2.5; ev

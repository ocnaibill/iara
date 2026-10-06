#!/usr/bin/env bash
# A janela com ENTRADA REAL: serviço + janela (via XWayland, GDK_BACKEND=x11) dirigida por xdotool (mouse e teclado reais) e
# pela acessibilidade (AT-SPI, o caminho de um leitor de tela). Cada verificação lê o efeito no serviço, não a aparência.
# Requisitos: xdotool, xwininfo, python-gobject + at-spi2-core (python do sistema), sox, PipeWire e sessão gráfica.
# ATENÇÃO: a janela do Iara aparece na tela e o ponteiro real se move durante o teste. O ponteiro só clica depois de confirmar
# que está sobre a janela do Iara. Só os fluxos de teste IaraTeste* são movidos; a saída padrão do sistema não é tocada.
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
for t in xdotool xwininfo sox pw-play busctl; do command -v $t >/dev/null || { echo "falta: $t"; exit 2; }; done
/usr/bin/python3 -c "import gi; gi.require_version('Atspi','2.0')" 2>/dev/null || { echo "falta python-gobject/at-spi2-core"; exit 2; }
(cd "$ROOT" && cargo build -q -p iara-service -p iara-ui --features gtk-ui) || exit 1
AT="/usr/bin/python3 $HERE/../janela/at.py"
TMP=$(mktemp -d); PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null; done; pkill -x iara-ui 2>/dev/null
  for id in $(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('application.name','').startswith('IaraTeste'): print(o['id'])" 2>/dev/null); do pw-metadata -n default -d "$id" target.object >/dev/null 2>&1; done
  rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state" IARA_BUS_NAME="dev.iara.MixerRI$$"
mkdir -p "$XDG_CONFIG_HOME/iara"
printf 'schema_version = 1\nautostart = true\nshare_output_device = false\nshare_microphone_device = false\ncapture_default_output = false\n' > "$XDG_CONFIG_HOME/iara/config.toml"
bc() { busctl --user call -- "$IARA_BUS_NAME" /dev/iara/Mixer dev.iara.Mixer1 "$@" 2>&1; }
state_json() { busctl --user --json=short call -- "$IARA_BUS_NAME" /dev/iara/Mixer dev.iara.Mixer1 GetState; }
PASS=0; FAIL=0
check() { if eval "$2"; then PASS=$((PASS+1)); echo "  ok    $1"; else FAIL=$((FAIL+1)); echo "  FALHA $1"; fi; }
py() { state_json | /usr/bin/python3 -c "
import json,sys,tomllib
d=json.load(sys.stdin)['data']; prof=tomllib.loads(d[1]); apps={a['display']:a for a in tomllib.loads(d[6]).get('app',[])}
$1"; }
app_is() { py "a=apps.get('$1',{}); print(a.get('channel','-')=='$2' and a.get('source')=='$3')" | grep -q True; }
gain() { py "c=[c for c in prof['channels'] if c['id']=='$1'][0]['$2']; print(-999 if c.get('silence') else c['gain_db'])"; }
flag() { py "c=[c for c in prof['channels'] if c['id']=='$1'][0]['$2']; print(c['$3'])"; }
profiles() { py "print(','.join(x[0] for x in json.loads(open('/dev/stdin').read())['data'][8]) if False else '')" >/dev/null 2>&1; state_json | /usr/bin/python3 -c "
import json,sys,tomllib
d=json.load(sys.stdin)['data']; print(tomllib.loads(d[1])['id'] + '|' + ','.join(x[0] for x in d[8]))"; }

"$ROOT/target/debug/iara-service" > "$TMP/svc.log" 2>&1 & PIDS+=($!)
for i in $(seq 1 100); do busctl --user status "$IARA_BUS_NAME" >/dev/null 2>&1 && break; sleep 0.1; done
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 600 sine 440 vol 0.02
for n in a b; do pw-play -P "{ node.name=IaraTeste$n application.name=IaraTeste$n application.process.binary=iarateste$n media.name=f$n }" "$TMP/t.wav" & PIDS+=($!); done
sleep 2
start_ui() { pkill -x iara-ui 2>/dev/null; sleep 1; GDK_BACKEND=x11 "$ROOT/target/debug/iara-ui" > "$TMP/ui.log" 2>&1 & sleep 3.5; }
win() { xdotool search --name "^Iara$" | head -1; }
origin() { local g; g=$(xdotool getwindowgeometry "$(win)" | awk '/Position/{print $2}'); OX=$(( ${g%%,*} + 14 )); OY=$(( ${g##*,} + 14 )); }
over_ui() { local w; w=$(xdotool getmouselocation --shell | awk -F= '/WINDOW/{print $2}'); [ "$w" = "$(win)" ] || xwininfo -id "$w" -tree 2>/dev/null | grep -q "Parent window id: $(printf '0x%x' "$(win)")"; }
safe_move() { origin; xdotool mousemove $((OX+$1)) $((OY+$2)); sleep 0.25; over_ui || { echo "  ABORTADO: ponteiro fora da janela do Iara"; return 1; }; }
sclick() { safe_move "$1" "$2" || return 1; xdotool mousedown 1; sleep 0.12; xdotool mouseup 1; sleep 0.7; }
sdrag() { safe_move "$1" "$2" || return 1; xdotool mousedown 1; sleep 0.2
  for s in 1 2 3 4 5 6 7 8 9 10; do xdotool mousemove $((OX+$1+($3-$1)*s/10)) $((OY+$2+($4-$2)*s/10)); sleep 0.06; done
  sleep 0.3; xdotool mouseup 1; sleep 1.6; }
chip_y() { local i; i=$($AT chips "$2" 2>/dev/null | grep -n "^$1$" | cut -d: -f1); echo $((536 + 32*(${i:-1}-1))); }
menu_open() { $AT dump label 2>/dev/null | grep -q PERFIS; }
ensure_menu() { for i in 1 2 3; do menu_open && return 0; sclick 672 22; sleep 0.5; done; menu_open; }

echo "== ambiente"; start_ui; origin
check "a janela do Iara abriu (XWayland)" '[ -n "$(win)" ]'
check "a acessibilidade enxerga a janela" '$AT dump label 2>/dev/null | grep -q MASTER'

echo "== etiquetas de aplicativos: arrastar e soltar com o mouse real"
Y=$(chip_y IaraTestea "NÃO ATRIBUÍDOS")
sdrag 60 "$Y" 520 300
check "arrastar IaraTestea para CHAT salva a regra" 'app_is IaraTestea chat rule'
check "o realce 'soltar aqui' sumiu (a coluna não ficou marcada)" '! $AT dump label 2>/dev/null | grep -q "drop-target"'
$AT act "Mover só nesta sessão" >/dev/null 2>&1
sdrag 473 536 920 300
check "com 'só nesta sessão', arrastar vale só até o app parar (origem=session)" 'app_is IaraTestea aux session'
check "...e a regra salva do perfil NÃO mudou" 'py "print([r[\"channel\"] for r in prof[\"rules\"]])" | grep -q "chat"'
sclick 873 536
check "clicar na etiqueta (sem arrastar) abre o menu 'Mover para…'" '$AT dump label 2>/dev/null | grep -q "Mover para GAME"'
$AT act "Reaplicar regra do perfil" >/dev/null 2>&1; sleep 2.5
check "'Reaplicar regra do perfil' volta à regra salva" 'app_is IaraTestea chat rule'
$AT act "Mover só nesta sessão" >/dev/null 2>&1

echo "== controles do mixer com o mouse e o teclado reais"
sdrag 280 228 280 313
g=$(gain game personal); check "arrastar o slider de escuta do GAME ao meio dá cerca de −30 dB ($g)" 'python3 -c "import sys; g=float(sys.argv[1]); sys.exit(0 if -40<g<-20 else 1)" "$g"'
sclick 280 313; g1=$(gain game personal)
for k in Up Up Up Up Up; do xdotool key $k; sleep 0.2; done; sleep 1; g2=$(gain game personal)
check "5 setas ↑ no slider = +5 dB exatos ($g1 → $g2)" 'python3 -c "import sys; d=float(sys.argv[2])-float(sys.argv[1]); sys.exit(0 if abs(d-5)<0.2 else 1)" "$g1" "$g2"'
xdotool key Prior; sleep 1; g3=$(gain game personal)
check "Page Up = +6 dB ($g2 → $g3)" 'python3 -c "import sys; d=float(sys.argv[2])-float(sys.argv[1]); sys.exit(0 if abs(d-6)<0.2 else 1)" "$g2" "$g3"'
sclick 546 474; check "clicar no mute da transmissão do CHAT muta" '[ "$(flag chat transmission muted)" = True ]'
sclick 940 188; check "clicar na participação da transmissão do AUX liga o envio" '[ "$(flag aux transmission enabled)" = True ]'
sclick 1127 169; check "mute global do MIC" 'py "print(prof[\"microphone\"][\"global_mute\"])" | grep -q True'
sdrag 657 654 900 654; check "arrastar o ChatMix para a direita" 'py "print(prof[\"chatmix\"][\"position\"] > 0.2)" | grep -q True'
sclick 1255 661; check "o botão Centro volta o ChatMix a 0" 'py "print(prof[\"chatmix\"][\"position\"] == 0.0)" | grep -q True'

echo "== acessibilidade: nomes"
n_sl=$($AT dump slider 2>/dev/null | grep -c slider); n_un=$($AT dump slider 2>/dev/null | sed 's/^ *//' | sort -u | wc -l)
check "todos os sliders têm nome próprio ($n_un nomes distintos em $n_sl sliders)" '[ "$n_sl" = "$n_un" ]'
check "o nome do slider diz o canal e o envio" '$AT dump slider 2>/dev/null | grep -q "GAME — volume da escuta"'
check "os botões de mute dizem o canal" '$AT dump "toggle button" 2>/dev/null | grep -q "GAME — silenciar escuta"'

echo "== perfis"
ensure_menu; check "o menu de perfis abre com o clique real" 'menu_open'
$AT dump "text,entry" >/dev/null 2>&1
/usr/bin/python3 - <<'PY'
import os,sys
sys.path.insert(0,os.path.join(os.environ.get('HERE',''),'..','janela'))
PY
HERE_AT="$HERE/../janela"; /usr/bin/python3 - "$HERE_AT" <<'PY'
import sys; sys.path.insert(0, sys.argv[1])
import at
for d,r,n,e,v,node in at.walk(at.app()):
    if v and node.get_editable_text_iface():
        node.get_editable_text_iface().set_text_contents("Teste"); break
PY
$AT act "Novo perfil" >/dev/null 2>&1; sleep 1.5
check "criar um perfil pelo menu" '[ "$(profiles | cut -d"|" -f2)" = "default,teste" ]'
ensure_menu; $AT act "Teste" >/dev/null 2>&1; sleep 3
check "trocar de perfil pelo menu" '[ "$(profiles | cut -d"|" -f1)" = "teste" ]'
check "o menu fecha sozinho depois de escolher um perfil" '! menu_open'
ensure_menu; $AT act "Excluir o perfil Padrão" >/dev/null 2>&1; sleep 0.6
sclick 672 22; sleep 0.6; ensure_menu; $AT act "Excluir o perfil Padrão" >/dev/null 2>&1; sleep 1.2
check "um clique em 'excluir' depois de fechar e reabrir NÃO exclui (confirmação desfeita)" '[ "$(profiles | cut -d"|" -f2)" = "default,teste" ]'
$AT act "Confirmar a exclusão do perfil Padrão" >/dev/null 2>&1; sleep 1.5
check "a segunda etapa (confirmação) exclui e manda para a lixeira" '[ "$(profiles | cut -d"|" -f2)" = "teste" ] && ls "$XDG_STATE_HOME"/iara/trash | grep -q default'

echo; echo "resultado: $PASS ok, $FAIL falhas"
[ "$FAIL" = 0 ]

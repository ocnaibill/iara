#!/usr/bin/env bash
# Medidores ao vivo de ponta a ponta com a JANELA real: serviço + janela contra um PipeWire e um D-Bus PRIVADOS (nada da sessão
# do usuário é tocado e a janela de teste vive numa tela virtual Xvfb: não aparece na tela de ninguém). Um seno conhecido toca num canal e
# o que cada barra mostra é lido da própria janela (gancho de desenvolvimento SIGUSR1 grava .png e .meters).
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
for t in xdotool sox pw-play busctl Xvfb; do command -v $t >/dev/null || { echo "falta: $t"; exit 2; }; done
(cd "$ROOT" && cargo build -q -p iara-service -p iara-ui --features gtk-ui) || exit 1
source "$HERE/lib_privado.sh"
TMP=$(mktemp -d); PIDS=(); PP=""
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null; done; [ -n "$PP" ] && kill "$PP" 2>/dev/null; rm -rf "$TMP"; privado_parar; }
trap cleanup EXIT
privado_subir || exit 1
privado_dbus || { echo "sem dbus-daemon"; exit 2; }
privado_tela || { echo "falta Xvfb (pacote xorg-server-xvfb)"; exit 2; }   # tela virtual: nada aparece na tela do usuário
mkdir -p "$XDG_CONFIG_HOME/iara"
printf 'schema_version = 1\nautostart = true\nshare_output_device = false\nshare_microphone_device = false\ncapture_default_output = false\n' > "$XDG_CONFIG_HOME/iara/config.toml"
BUS=dev.iara.Mixer
bc() { busctl --user call -- "$BUS" /dev/iara/Mixer dev.iara.Mixer1 "$@" 2>&1; }
PASS=0; FAIL=0
check() { if eval "$2"; then PASS=$((PASS+1)); echo "  ok    $1"; else FAIL=$((FAIL+1)); echo "  FALHA $1  [$3]"; fi; }
taps() { pw-cli ls Node | grep -c 'node.name = "iara\.meter\.' || true; }
near() { awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN{ d=a-b; if (d<0) d=-d; exit !(d<=t) }'; }
pos() { awk -v d="$1" 'BEGIN{ p=(d+60)/60; if (p<0) p=0; if (p>1) p=1; printf "%.4f", p }'; }  # dBFS → posição 0..1
SHOT="$TMP/shot.png"
snap() { local a b;  a=$(stat -c %y "$SHOT" "$TMP/shot.meters" 2>/dev/null | tr -d '\n'); kill -USR1 "$UIPID"; sleep 1.5
  b=$(stat -c %y "$SHOT" "$TMP/shot.meters" 2>/dev/null | tr -d '\n'); [ "$a" != "$b" ] || echo "  (aviso: captura não atualizada)"
  local f; f=$(cat "$TMP"/ui*.log 2>/dev/null | grep -c falha); [ "$f" = "${LASTF:-0}" ] || { echo "  (aviso: a captura falhou: $(grep -h falha "$TMP"/ui*.log | tail -1)) janela: $(xdotool search --pid "$UIPID" 2>/dev/null | while read -r w; do echo -n "$w=$(xwininfo -id "$w" 2>/dev/null | awk '/Map State/{print $3}')/$(xwininfo -id "$w" 2>/dev/null | awk '/Width/{print $2}') "; done)"; }; LASTF=$f; }
val() { awk -v k="$1" -v c="$2" '$1==k {print $c}' "$TMP/shot.meters"; }   # coluna 2 = nível, 3 = pico, 4 = clipe
tone() { pw-play --target "$1" "$TMP/$2.wav" & PP=$!; sleep "${3:-2}"; }
stop_tone() { [ -n "$PP" ] && kill "$PP" 2>/dev/null; wait "$PP" 2>/dev/null; PP=""; }

sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/half.wav" synth 40 sine 440 vol 0.5
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/hot.wav" synth 40 sine 440 vol 1.2

"$ROOT/target/debug/iara-service" > "$TMP/svc.log" 2>&1 & PIDS+=($!)
for _ in $(seq 1 100); do busctl --user status "$BUS" >/dev/null 2>&1 && break; sleep 0.1; done
sleep 5   # primeira aplicação do perfil
echo "# sem janela: nenhum tap no grafo"
check "zero taps com o serviço sozinho" "[ \"\$(taps)\" = 0 ]" "$(taps)"

echo "# janela aberta: os medidores ligam sozinhos"
IARA_UI_SHOT_PATH="$SHOT" "$ROOT/target/debug/iara-ui" > "$TMP/ui.log" 2>&1 & UIPID=$!; PIDS+=($UIPID)
sleep 5
check "oito taps com a janela aberta" "[ \"\$(taps)\" = 8 ]" "$(taps)"
snap
check "em silêncio todas as barras no piso" "[ \"\$(awk '\$2>0 || \$4==\"true\"' \"\$TMP/shot.meters\" | wc -l)\" = 0 ]" "$(awk '$2>0' "$TMP/shot.meters" | head -3)"

echo "# seno 0,5 (−6,02 dBFS) no canal GAME: as barras de GAME acendem na altura certa, as outras não"
tone iara.ch.game half 2; snap; stop_tone
exp=$(pos -6.02)
for k in ch:game:personal ch:game:transmission; do
  check "$k nível ≈ −6,02 dBFS" "near \"\$(val $k 2)\" $exp 0.02" "$(val $k 2) esperado $exp"
  check "$k pico mantido na mesma altura" "near \"\$(val $k 3)\" $exp 0.02" "$(val $k 3)"
  check "$k sem clipe" "[ \"\$(val $k 4)\" = false ]" "$(val $k 4)"
done
check "chat em silêncio" "near \"\$(val ch:chat:personal 2)\" 0 0.001" "$(val ch:chat:personal 2)"
check "MASTER escuta ≈ −6,02 (único canal tocando)" "near \"\$(val master:personal 2)\" $exp 0.03" "$(val master:personal 2)"
cp "$SHOT" "$TMP/../iara-medidores-game.png" 2>/dev/null

echo "# escuta −12 dB: só a barra da escuta cai (pós-ganho); a transmissão e o barramento ficam"
bc SetChannelGain ssd game personal -12 >/dev/null; sleep 1
tone iara.ch.game half 2; snap; stop_tone
check "escuta ≈ −18,04 dBFS" "near \"\$(val ch:game:personal 2)\" $(pos -18.04) 0.02" "$(val ch:game:personal 2)"
check "transmissão segue ≈ −6,02" "near \"\$(val ch:game:transmission 2)\" $exp 0.02" "$(val ch:game:transmission 2)"
bc SetChannelGain ssd game personal 0 >/dev/null

echo "# mute da escuta: a barra vai a zero na hora"
bc SetChannelMute ssb game personal true >/dev/null; sleep 1
tone iara.ch.game half 2; snap; stop_tone
check "escuta mutada em zero" "near \"\$(val ch:game:personal 2)\" 0 0.001" "$(val ch:game:personal 2)"
check "transmissão segue" "near \"\$(val ch:game:transmission 2)\" $exp 0.02" "$(val ch:game:transmission 2)"
bc SetChannelMute ssb game personal false >/dev/null

echo "# depois que o som para, nível e pico decaem até o piso e o relógio da janela se desliga"
sleep 4; snap
check "nível no piso" "near \"\$(val ch:game:transmission 2)\" 0 0.001" "$(val ch:game:transmission 2)"
check "pico solto" "near \"\$(val ch:game:transmission 3)\" 0 0.001" "$(val ch:game:transmission 3)"

echo "# sinal acima de 0 dBFS: indicador de clipe aceso e barra no topo"
tone iara.ch.media hot 2; snap; stop_tone
check "MEDIA escuta clipa" "[ \"\$(val ch:media:personal 4)\" = true ]" "$(val ch:media:personal 4)"
check "MEDIA escuta no topo" "near \"\$(val ch:media:personal 2)\" 1 0.01" "$(val ch:media:personal 2)"
cp "$SHOT" "$TMP/../iara-medidores-clipe.png" 2>/dev/null
sleep 3; snap
check "clipe apaga depois de 2 s" "[ \"\$(val ch:media:personal 4)\" = false ]" "$(val ch:media:personal 4)"

echo "# canal criado e removido com a janela aberta: o tap acompanha"
bc AddChannel ss musica Musica >/dev/null; sleep 3
check "nove taps com o canal novo" "[ \"\$(taps)\" = 9 ]" "$(taps)"
tone iara.ch.musica half 2; snap; stop_tone
check "o canal novo mostra o seno na escuta" "near \"\$(val ch:musica:personal 2)\" $exp 0.02" "$(val ch:musica:personal 2)"
bc RemoveChannel ss musica "" >/dev/null; sleep 8
check "de volta a oito taps depois de remover" "[ \"\$(taps)\" = 8 ]" "$(taps)"

echo "# janela minimizada: medidores desligam no serviço; ao voltar, religam"
WID=$(xdotool search --onlyvisible --pid "$UIPID" --name "^Iara$" | head -1)
xdotool windowminimize "$WID"; sleep 2
check "zero taps com a janela minimizada" "[ \"\$(taps)\" = 0 ]" "$(taps)"
xdotool windowmap "$WID"; xdotool windowactivate "$WID" 2>/dev/null; sleep 2
check "oito taps ao voltar" "[ \"\$(taps)\" = 8 ]" "$(taps)"

echo "# janela some sem avisar (kill -9): o serviço solta o pedido sozinho"
kill -9 "$UIPID"; sleep 2
check "zero taps depois que a janela some" "[ \"\$(taps)\" = 0 ]" "$(taps)"

echo "# janela volta depois de reiniciar o serviço: o pedido é refeito"
kill -TERM "${PIDS[0]}"; sleep 2
"$ROOT/target/debug/iara-service" > "$TMP/svc2.log" 2>&1 & PIDS[0]=$!
IARA_UI_SHOT_PATH="$SHOT" "$ROOT/target/debug/iara-ui" > "$TMP/ui2.log" 2>&1 & UIPID=$!; PIDS+=($UIPID)
sleep 8
check "oito taps com janela nova" "[ \"\$(taps)\" = 8 ]" "$(taps)"
kill -TERM "${PIDS[0]}"; sleep 1.5; "$ROOT/target/debug/iara-service" > "$TMP/svc3.log" 2>&1 & PIDS[0]=$!
sleep 9
check "serviço reiniciado com a janela aberta: taps de volta" "[ \"\$(taps)\" = 8 ]" "$(taps)"
tone iara.ch.game half 2; snap; stop_tone
check "e os níveis voltam a chegar" "near \"\$(val ch:game:transmission 2)\" $exp 0.02" "$(val ch:game:transmission 2)"

check "a janela não registrou falhas de captura" "! grep -q 'falha' \"\$TMP/ui.log\" \"\$TMP/ui2.log\"" "$(grep -h falha "$TMP/ui.log" "$TMP/ui2.log" | head -2)"
echo "passou: $PASS  falhou: $FAIL"
[ "$FAIL" = 0 ]

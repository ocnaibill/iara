#!/usr/bin/env bash
# Lança o serviço e a janela do Iara para um teste feito por uma pessoa.
#
#   tools/teste-humano.sh             modo ISOLADO (padrão): configuração própria em ~/.local/share/iara-teste, nome de D-Bus
#                                     de teste, NÃO troca a saída padrão do sistema. A saída preferida é o seu dispositivo padrão
#                                     atual (fixada na primeira vez), então um aplicativo que você arrastar para um canal
#                                     continua saindo pelo mesmo fone, agora pelo mixer.
#   tools/teste-humano.sh --completo  modo COMPLETO: sua configuração real (~/.config/iara) e o comportamento de produção,
#                                     inclusive tornar "Iara — Saída principal" a saída padrão do sistema (pede confirmação).
#                                     Se algo der errado:  cargo run -p iara-service -- --restore-default
#
# Ao fechar a janela, o serviço é encerrado e tudo que o Iara criou some (os aplicativos voltam à saída física).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cd "$ROOT" && cargo build -q -p iara-service -p iara-ui --features gtk-ui || exit 1

if [ "${1:-}" = "--completo" ]; then
  echo "MODO COMPLETO: o Iara vai virar a saída padrão do sistema e usar ~/.config/iara."
  echo "Seus aplicativos que seguem o padrão passarão pelo mixer. Recuperação: cargo run -p iara-service -- --restore-default"
  read -r -p "Continuar? [s/N] " r; [ "$r" = s ] || [ "$r" = S ] || exit 0
  MODE=completo
else
  BASE="$HOME/.local/share/iara-teste"
  export XDG_CONFIG_HOME="$BASE/config" XDG_STATE_HOME="$BASE/state" IARA_BUS_NAME="dev.iara.MixerTeste"
  mkdir -p "$XDG_CONFIG_HOME/iara" "$XDG_STATE_HOME"
  [ -f "$XDG_CONFIG_HOME/iara/config.toml" ] || printf 'schema_version = 1\nautostart = true\nshare_output_device = false\nshare_microphone_device = false\ncapture_default_output = false\n' > "$XDG_CONFIG_HOME/iara/config.toml"
  MODE=isolado
fi

SVC=""
cleanup() { [ -n "$SVC" ] && kill -TERM "$SVC" 2>/dev/null && wait "$SVC" 2>/dev/null; echo "serviço encerrado."; }
trap cleanup EXIT INT TERM
"$ROOT/target/debug/iara-service" &
SVC=$!
NAME="${IARA_BUS_NAME:-dev.iara.Mixer}"
for _ in $(seq 1 100); do busctl --user status "$NAME" >/dev/null 2>&1 && break; sleep 0.1; done

if [ "$MODE" = isolado ] && [ ! -f "$XDG_STATE_HOME/saida-semeada" ]; then
  cur=$(pw-metadata -n default 0 default.configured.audio.sink 2>/dev/null | grep -o 'name":"[^"]*' | head -1 | cut -d'"' -f3)
  if [ -n "$cur" ]; then
    sleep 1
    busctl --user call -- "$NAME" /dev/iara/Mixer dev.iara.Mixer1 SetPreferredOutput s "$cur" >/dev/null 2>&1 && echo "saída preferida: $cur" && touch "$XDG_STATE_HOME/saida-semeada"
  fi
fi

cat <<'GUIA'

== O QUE TESTAR (e me contar o que estranhar) ==
 1. Toque algo no Zen/Cider. Ele aparece em NÃO ATRIBUÍDOS (coluna MASTER)? Arraste a etiqueta para um canal
    (ou clique nela e use "Mover para…"). O som continua saindo? Com alguma mudança de volume?
 2. Arraste os sliders, use as setas do teclado (1 dB) e Page (6 dB), clique nos mutes e nos botões de escuta/transmissão.
 3. Menu do perfil (cabeçalho): crie, duplique, renomeie, troque e exclua perfis.
 4. Engrenagem de um canal: renomear e remover. "+" cria canal.
 5. "Mover só nesta sessão" (cabeçalho): arraste um app com ele ligado e depois use "Reaplicar regra do perfil".
 6. Desconecte/reconecte o fone ou o microfone: o aviso aparece e volta ao normal?
 7. Teclado puro: Tab até uma etiqueta de aplicativo, Enter para abrir o menu, setas e Enter para escolher.
 8. Ache o que for estranho de olhar, de clicar, de ler. Quanto mais específico, melhor.
Feche a janela para encerrar tudo.
GUIA
"$ROOT/target/debug/iara-ui"

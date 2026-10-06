# Registro técnico das provas (seção 6.3.2)

Resultados medidos, com comando e versões. Só vale o que está medido aqui.

## Ambiente

- 2026-10-05 · PipeWire 1.6.9 (daemon da sessão do usuário, acessado de um toolbox Arch) · GTK 4.22.5.
- Ferramentas: `pw-cli`, `pw-loopback`, `pw-play`, `pw-record`, `sox` (medição de RMS).
- Distribuição/WirePlumber do host não registrados ainda.

## Prova 01 — hipótese A: sink de canal + dois loopbacks do monitor

Script: `tools/provas/01-ramos-loopback.sh`. Cria um sink nulo de canal e dois sinks de destino
(pessoal, transmissão), liga cada destino ao monitor do canal por um `pw-loopback`, toca um
seno de 440 Hz (amplitude 0,5, 48 kHz estéreo) no canal e grava os dois destinos ao mesmo tempo.
O ganho é aplicado por `channelVolumes` no nó de saída de cada loopback.

| Cenário | Pessoal (dBFS RMS) | Transmissão (dBFS RMS) |
| --- | --- | --- |
| Base (ganhos 1,0) | −9,27 | −9,23 |
| Pessoal −6 dB | −15,23 (Δ −5,96) | −9,27 (Δ −0,04) |
| Transmissão −6 dB | −9,27 | −15,28 (Δ −6,05) |
| Transmissão com ganho 0 | −9,23 | −∞ |

Critério do passo 2 (±0,2 dB no ramo alterado; <0,1 dB no outro): **atendido** nas duas direções.
Limpeza: após a execução não restou nenhum nó `iara_proof_*` e os dispositivos padrão não mudaram.

## Prova 02 — dois canais, MASTER após a soma, Não atribuídos e MIC

Script: `tools/provas/02-mixes-master-mic.sh`. Dois canais (A, B) com envio pessoal/transmissão cada,
somados em dois sinks de mix; um loopback de cada mix leva ao MASTER correspondente (ganho depois da soma).
Não atribuídos entra só no mix pessoal. MIC (simulado por sink nulo) passa por ganho/mute global
e se divide em fonte dedicada para aplicativos, envio pessoal e envio de transmissão.
Mute é a flag `mute` do nó (`Props`), não ganho 0. Valores em dBFS RMS.

| Cenário | MASTER pessoal | MASTER transmissão | MIC apps |
| --- | --- | --- | --- |
| A sozinho (uma cópia) | −9,23 | −9,27 | — |
| A + B (440 e 880 Hz) | −6,17 | −6,17 | — |
| A pessoal −6 dB | −15,23 | −9,27 | — |
| A transmissão mute (flag) | −9,27 | −∞ | — |
| MASTER pessoal −6 dB (A+B) | −12,22 | −6,22 | — |
| MASTER transmissão −6 dB (A+B) | −6,22 | −12,22 | — |
| Não atribuídos sozinho | −9,18 | −∞ | — |
| MIC sozinho | −9,08 | −9,08 | −9,08 |
| MIC: mute do envio de transmissão | −9,13 | −∞ | −9,13 |
| MIC: mute do envio pessoal | −∞ | −9,13 | −9,13 |
| MIC: mute global | −∞ | −∞ | −∞ |
| MASTER pessoal −20 dB | −29,13 | −9,13 | −9,18 (intocado) |

Leitura: a soma de dois tons distintos dá +3,0 dB (potência), e uma só fonte mede o mesmo nível que a
base da prova 01, sem acréscimo de +6 dB — **sem cópia duplicada** neste cenário. Mute por flag e
ganho por ramo são independentes; MASTER age só no seu mix; Não atribuídos não chega à transmissão;
o ramo dedicado do MIC não sofre o MASTER nem os mutes dos envios, e o mute global silencia os três.
Limpeza: nenhum nó `iara_proof_*` restou.

## Prova 03 — nós hospedados por um processo Rust (bindings `pipewire` 0.10.1)

Código: `tools/pw-probe` (crate descartável, fora do workspace) e `tools/provas/03-hospedagem-rust.sh`.
O processo Rust conecta ao PipeWire, cria sinks nulos com `core.create_object("adapter", …)`, carrega
`libpipewire-module-loopback` **no próprio contexto** (via `pipewire::sys::pw_context_load_module`, unsafe;
não há wrapper seguro), liga os nós pelo registro e aplica `channelVolumes`/`mute` com `Node::set_param(Props)`.

| Cenário | Pessoal | Transmissão |
| --- | --- | --- |
| Base | −9,23 | −9,23 |
| Pessoal −6 dB (Props) | −15,28 | −9,27 |
| Transmissão mute (flag, Props) | −9,27 | −∞ |

- Build: `pipewire`/`pipewire-sys` 0.10.1 compilam contra libpipewire 1.6.9 (precisa de clang/bindgen).
- Hospedagem no contexto do serviço: ao enviar SIGKILL ao processo, **nenhum** nó `iara_probe_*` restou
  (0 nós no grafo). Logo não há órfãos, mas **o áudio dos canais some junto com o serviço**; o comportamento dos
  aplicativos nesse momento (fallback do WirePlumber) ainda não foi observado.

### Achado: o WirePlumber restaura volume/mute dos nós do próprio Iara

`wireplumber` 0.5.18 grava em `~/.local/state/wireplumber/stream-properties` o volume e o mute de streams e sinks
(chave `media.name` ou `node.name`) e os restaura quando o nó reaparece. Numa execução anterior o script
foi encerrado com mute ativo; nas execuções seguintes o ramo de transmissão já nascia mudo (−∞ na base, 4 de 4
repetições) mesmo com os links corretos. Opt-out por nó, confirmado: `state.restore-props=false` e
`state.restore-target=false` nas propriedades do sink e dos streams do loopback; com o estado sujo
(`mute:true`) ainda salvo, a base voltou a −9,23/−9,23 em 2 de 2 execuções.
Consequência para o produto: **todos os nós criados pelo Iara devem declarar esse opt-out**; o estado desejado
é do Iara, não do WirePlumber (spec 6.3: não disputar políticas globais).

## Prova 04 — queda do processo, reinício do PipeWire e hospedagem no daemon

Scripts: `tools/provas/04a-queda-do-processo.sh`, `04b-reinicio-pipewire.sh`, `04c-linger.sh`
(sonda `tools/pw-probe`). Streams de teste: `pw-play` com destino explícito num sink do Iara (S1) e sem destino (S2).
O reinício do PipeWire foi feito de verdade na sessão do usuário (`systemctl --user restart pipewire.service`).

**04a — SIGKILL do processo proprietário (2 execuções, mesmo resultado)**

| Momento | S1 (destino explícito `iara_probe_chan`) | S2 (sem destino) |
| --- | --- | --- |
| Processo vivo | → `iara_probe_chan` | → saída física padrão |
| 3 s após SIGKILL | → **saída física** (fallback) | → saída física |
| Nós recriados (4 s) | continua na saída física | igual |
| `pw-metadata … target.object=iara_probe_chan` | volta ao `iara_probe_chan` em ≤2 s | igual |

Leitura: a queda do serviço faz o fluxo cair na saída física (vaza para os alto-falantes) e **o fluxo não volta
sozinho** quando o nó reaparece; o serviço precisa reencaminhar por metadata ao reiniciar. Metadata funciona para isso.

**04b — reinício do PipeWire (1 execução)**
- Todos os nós do processo sumiram do grafo (0 de 11), WirePlumber voltou ativo, dispositivos padrão preservados
  (mesmos nomes; IDs renumerados) e os aplicativos da sessão (Zen, Cider) reconectaram à saída física.
- O processo Rust **continuou vivo** e ainda acreditava que seus nós existiam (a sonda não ouve o erro do core):
  sem tratamento de desconexão, o serviço ficaria “saudável” e sem áudio. É requisito: ouvir o erro do core,
  invalidar proxies, reconectar e reconstruir sem duplicar (spec 8.8).
- Tempo de indisponibilidade **não medido** (o `printf` do script usou o locale errado e o número é inválido).

**04c — `object.linger` (nó criado no daemon via `create_object("adapter")`)**
- Sem linger: o nó some junto com o processo. Com `object.linger=true`: **sobrevive ao SIGKILL** (removido depois
  à mão com `pw-cli destroy`). Logo buses/sinks podem ficar no daemon, mas viram **órfãos** se ninguém os limpar:
  o serviço precisa marcar seus objetos (prefixo/propriedade própria) e varrê-los na inicialização.
- Os ramos com ganho (loopbacks) vivem no processo cliente e não têm equivalente com linger nesta prova. Hospedar tudo no
  daemon por `pipewire.conf.d` **não foi testado** (exigiria reinícios extras e não permite canais dinâmicos sem recarregar).

## Prova 05 — mover aplicativos e ausência de dispositivo físico

Scripts: `tools/provas/05a-mover-aplicativos.sh`, `05b-ausencia-dispositivo.sh`, `lib_links.py` (sonda `tools/pw-probe`, comando `lpx`).

**05a — mover por metadata (streams `pw-play` de teste)**

| Caso | Resultado |
| --- | --- |
| A (sem destino): `pw-metadata … target.object=<nome do sink>` | moveu para o sink do Iara |
| B com `node.dont-move=true` | **não moveu**; a propriedade é visível no nó antes da tentativa (detectável) |
| C com destino explícito na saída física (`--target`) | moveu (metadata vence o destino explícito do stream) |
| Mudança externa em A com `pw-metadata` | A voltou à saída física; um observador independente (`pw-metadata -m`) viu `update id:<A> key:'target.object'` com o nome |
| Mudança externa com `pactl move-sink-input` (protocolo Pulse) | A moveu; o observador viu `target.node=<id do nó>` e `target.object=<serial numérico>` (formato diferente do nosso, que é o nome) |

Leitura: movimentos externos são observáveis como eventos de metadata cujo sujeito é o id do stream; escritas próprias
são reconhecíveis por quem as fez. Não se mediu a política de aplicativos que ignoram metadata sem `dont-move`.
Não explicado: na segunda execução o stream A já nasceu ligado ao sink do Iara, sem pedido. Hipótese não confirmada:
restauração de destino do WirePlumber ou metadata remanescente; o arquivo de estado não contém entrada para A.
Da leitura de `state-stream.lua` (não medido): o WirePlumber grava o destino ao ver `target.object` mudar quando ele
resolve o nó pelo **serial numérico** (caso do `pactl`/seletores do desktop), e a escrita por nome (a nossa) não o resolve.

**05b — ausência e retorno do fifine AM8 Pro (perfil de placa desligado e religado por software)**

Quatro loopbacks (`nofb` = com `node.dont-fallback`; `fb` = sem) do microfone físico para um sink do Iara e de um sink do Iara para
a saída física do fifine. Resultado após 4 s de ausência e 6 s após o retorno (1 execução):

| Variante | Ausente | Após o retorno |
| --- | --- | --- |
| Entrada `nofb` | nó do loopback **sumiu** do grafo (sem fallback) | **não voltou** (continua ausente) |
| Entrada `fb` | **caiu para outro microfone** (webcam C922) | voltou ao fifine sozinha |
| Saída `nofb` | nó do loopback sumiu (sem fallback para as caixas) | **não voltou** |
| Saída `fb` | **caiu para as caixas (ALC887)** | voltou ao fifine sozinha |

- Fonte padrão do sistema: mudou para a C922 durante a ausência e voltou para o fifine sozinha.
- O módulo loopback encerra o par de streams quando o destino/origem some com `dont-fallback`; o processo cliente seguiu vivo.
- Consequência: nem o loopback com fallback (troca indevida de dispositivo, proibida pelas specs 8.5/8.6) nem o sem fallback (some e não volta)
  atendem sozinhos. O serviço precisa ser dono do ciclo: manter os nós virtuais estáveis (silêncio sem origem), observar o registro
  e **recriar** o par de streams quando o dispositivo preferido voltar, com a geração da escolha (8.5).
- Efeito na sessão do usuário durante o teste: o stream de captura do Zen terminou na C922 depois da ausência do fifine. A causa **não está
  estabelecida**: pode ter sido o fallback do WirePlumber, o navegador reagindo à remoção do dispositivo, ou o próprio usuário trocando o microfone
  no Google Meet na mesma hora (o usuário relatou essa possibilidade). O que está medido é só o comportamento dos loopbacks de teste e da fonte padrão.
  Por precaução o Zen foi movido ao fifine por metadata (uma chave `target.object` do Zen permanece no daemon até o próximo reinício do PipeWire);
  isso pode ter sobrescrito uma escolha deliberada do usuário. Uma limpeza minha apagou essa chave uma vez por engano e ela foi regravada.
  O perfil do fifine foi restaurado ao original.

## Prova 06 — latência, CPU, RAM e xruns (passo 8)

Scripts: `tools/provas/06a-latencia.sh`, `06b-recursos.sh`. PipeWire 1.6.9, `clock.rate=48000`, `clock.quantum=1024`
(21,3 ms por ciclo), grafo guiado pelo driver ALC887 da sessão do usuário, que seguia com Zen e Cider ativos (ruído de base).

**06a — latência de um clique entre sinks de uma cadeia de loopbacks hospedados no processo Rust (5 repetições por caminho)**

| Caminho | Atraso (ms) |
| --- | --- |
| Controle positivo: loopback com `-d 0,05` s | 50,0 / 50,0 / 50,0 / 50,0 / 50,0 |
| canal → mix (1 loopback) | 0,0 em todas |
| canal → MASTER (2 em série) | 0,0 em todas |
| MIC → fonte para aplicativos (2 em série) | 0,0 em todas |
| MIC → MASTER de transmissão (3 em série) | 0,0 em todas |

Método: dois taps simétricos copiam a origem para FL e o fim do caminho para FR de um sink estéreo; o atraso é a diferença entre
os picos (resolução de 1 amostra ≈ 0,02 ms). O controle de 50 ms foi lido corretamente, então a ferramenta enxerga atrasos.
Leitura: neste grafo (um único driver, nós ligados) os estágios de loopback não adicionam atraso mensurável entre sinks.
Isto **não** inclui a latência de saída do dispositivo físico, nem o comportamento com outro quantum (aplicativos que pedem quantum
menor/maior) ou com Bluetooth.

**06b — recursos com a topologia completa da prova 02 (11 loopbacks no processo Rust, 54 nós no grafo)**

| Estado | CPU pipewire | CPU wireplumber | CPU sonda | RSS pipewire | RSS sonda |
| --- | --- | --- | --- | --- | --- |
| Base (sem topologia, 15 s) | 1,2 % | 0,0 % | — | 31,3 MiB | — |
| Topologia ociosa (20 s) | 2,0 % | 0,1 % | 1,0 % | 115,6 MiB | 24,2 MiB |
| 3 tons ativos (20 s) | 1,6 % | 0,0 % | 0,6 % | 125,5 MiB | 24,4 MiB |

- CPU em % de um núcleo, de janelas de 20 s (contadores de `/proc`); xruns (`pw-top`, coluna ERR cumulativa): **0** nos 32 nós do Iara e
  no driver principal, nessas janelas curtas.
- RSS do `pipewire` sobe ~85–95 MiB com a topologia (≈1,7 MiB por nó, a investigar se os buffers são dimensionáveis) e **volta a 31,5 MiB**
  depois da remoção (pico 123,5 MiB): sem vazamento observado.
- Sem metas numéricas definidas; estes são os números de partida para a spec 10.

## Prova 07 — motor real (`iara-audio`) de ponta a ponta, e defeito do PipeWire com `Audio/Source/Virtual`

Script: `tools/provas/07-motor-e2e.sh` (exemplo `mixer_cli`, perfil inicial). 42 nós `iara.*` (12 nós, 13 ramos × 2, 2 fontes × 2); 15
reaplicações sem duplicar; 0 nós após encerrar o motor. Resultados nos cenários da spec 12 (dBFS RMS, seno 0,5):

| Cenário | MASTER pessoal | MASTER transmissão | Fonte Transmissão | Fonte Microfone |
| --- | --- | --- | --- | --- |
| GAME (base) | −9,27 | −9,23 | −9,23 | −∞ |
| GAME escuta −6 dB | −15,23 | −9,23 | — | — |
| MEDIA fora da transmissão | −9,18 | −∞ | — | — |
| Não atribuídos | −9,04 | −∞ | — | — |
| MASTER transmissão −6 dB (GAME) | −9,23 | −15,18 | −15,27 | −∞ |
| ChatMix +1: GAME / CHAT | −∞ / −9,08 | −9,23 / −9,08 | — | — |
| MIC (base) | −∞ | −9,08 | −9,08 | −9,08 |
| MIC mute global | −∞ | −∞ | −∞ | −∞ |
| MASTER pessoal −20 dB (MIC) | −∞ | −9,13 | −9,04 | −9,08 (intocado) |

Fontes lidas por um cliente de captura (`pw-record --target`, sem o recurso de monitor) como o OBS/Discord leem. O OBS do usuário roda no host,
fora do distrobox, mas usa o mesmo daemon; a seleção dentro do OBS não foi testada.

**Defeito do PipeWire 1.6.9 (reproduzido no distrobox e no host Bazzite):** qualquer `pw-loopback` com `media.class=Audio/Source/Virtual` no lado de
reprodução termina com SIGSEGV (código 139) — inclusive o padrão canônico (sink + source virtual) e sem nenhum nó do Iara. O log (`PIPEWIRE_DEBUG=4`) mostra
`impl-node.c:1893 node_port_info(): can't add port …: -28` (ENOSPC) imediatamente antes da queda; gdb: SIGSEGV em libpipewire, chamado por
`libspa-audioconvert` a partir de `module-client-node`. Com `media.class=Audio/Source` (sem `Virtual`) o mesmo loopback funciona e aparece como fonte. Decisão: as fontes
do Iara usam `Audio/Source`. Sentinela: `tools/provas/08-bug-source-virtual.sh`. Defeito não reportado a montante.

## Prova 09 — dispositivos físicos no motor real (implementa a spec 6.3.1.3)

Scripts: `tools/provas/09-dispositivos-e2e.sh` (HDMI como saída e placa de captura ezcap como entrada, que não têm outros usuários) e
`10-dispositivos-reais.sh` (fone na saída analógica da placa-mãe e microfone USB do desenvolvedor, só conferência de ligações).
O motor reconcilia, a cada 100 ms e a cada evento do registro, as ligações `iara.dev.output` (MASTER pessoal → dispositivo) e `iara.dev.input`
(dispositivo → entrada do MIC): cria quando o dispositivo existe, remove quando some, espera ≥2 s entre tentativas e nunca troca de dispositivo.
O módulo loopback se descarrega sozinho com `dont-fallback`; um listener do evento `destroy` do módulo evita destruí-lo duas vezes.

| Etapa | Resultado |
| --- | --- |
| Preferências definidas | ligações criadas; tom de 0,03 chega ao monitor do HDMI em −33,57 dBFS (esperado ≈ −33,5) |
| HDMI some (perfil off) | ligação removida; evento `DeviceAbsent`; MASTER pessoal sem destino (**nenhum fallback** para outro sink) |
| HDMI volta | ligação recriada sozinha em ≤7 s; tom −33,62 dBFS; evento `DeviceBack` |
| ezcap some e volta (entrada) | idem, com `DeviceAbsent`/`DeviceBack` |
| Usuário escolhe “nenhuma saída” durante a ausência | na volta **não** recria a ligação antiga |
| Fim | 0 nós `iara.*`; perfis das placas restaurados |
| Dispositivos reais | saída do Iara coexiste com Cider e Zen no mesmo sink; microfone compartilhado com o Zen |

Limites: a chave do dispositivo é o `node.name` (identificação persistente em aberto, spec 14); a escolha de alternativa autorizada, a geração da
escolha e a política de “não forçar reprodução” ficam para o serviço; o motor só informa presença/ausência.

## Prova 11 — serviço real: início, reinício do PipeWire e encerramento

Script: `tools/provas/11-servico-e2e.sh` (diretórios XDG isolados; reinicia o PipeWire da sessão do usuário).
O serviço (`iara-service`) carrega ou cria o perfil ativo, conecta o motor, aplica o plano e espera num único canal por comandos e eventos
do motor, acordando só para prazos (autosave 300 ms, fechamento de gesto 2 s, reconexão com espera 1/2/4/8/10 s).

| Etapa | Resultado |
| --- | --- |
| Início | 42 nós em 0,1 s; `default.toml` criado no diretório isolado |
| `systemctl --user restart pipewire` | comando durou 0,1 s; saída física amostrada ausente por ~0,2 s (amostragem de 100 ms) |
| Reconexão do serviço | nós do Iara abaixo de 42 de 0,0 s a 1,0 s; **de volta a 42 em 1,1 s**; máximo amostrado 42 (**sem duplicar**); serviço vivo |
| Log | `conexão com o PipeWire perdida; reconectando` → `reconectado ao PipeWire; reaplicando o perfil` |
| SIGTERM | código de saída 0; 0 nós `iara.*` |

Os testes unitários (`cargo test -p iara-service`) cobrem, com relógio sintético e backend falso: perfil padrão criado/reaproveitado e nunca sobrescrito se
ilegível, backoff da conexão, reaplicação do **perfil atual** após queda (por evento ou por falha do `apply`), autosave e agrupamento do histórico (20 passos de slider =
1 revisão), revisão estrutural imediata, flush ao desligar, perfil inválido recusado e falha de gravação visível no estado.

Limites: o reinício de PipeWire testado é o do systemd (rápido por ativação de socket); queda abrupta do daemon, reinícios repetidos e o reencaminhamento dos fluxos
associados por regra (não há regras ainda) não foram exercitados. Sem IPC: o serviço ainda não aceita comandos de uma interface.

## Prova 12 — IPC D-Bus de ponta a ponta, com cliente independente

Script: `tools/provas/12-ipc-e2e.sh` (serviço real, PipeWire real, barramento de sessão real, diretórios XDG isolados, nome de teste via
`IARA_BUS_NAME`; o cliente é o `busctl`, não o nosso código). Testes de integração do crate: `cargo test -p iara-ipc` (5, contra o barramento real;
sem barramento eles se declaram ignorados).

| Etapa | Resultado |
| --- | --- |
| Interface publicada | 18 métodos e 1 sinal (`busctl introspect`), assinatura de `GetState` = `tsbsasu` |
| `SetChannelGain game personal -6` | resposta `t 4`; GAME → MASTER pessoal passou de −9,27 para **−15,27 dBFS** (−6,00 dB reais) |
| mesmo comando com `-inf` | −∞ medido (silêncio exato) |
| Argumentos inválidos | erros D-Bus com a razão: envio inválido, ChatMix fora da faixa, id `../x` |
| `AddChannel musica` | 45 nós (42 + 1 sink + 2 streams do envio pessoal); transmissão do canal novo desligada |
| Segunda instância, mesmo nome | recusada (código 1, "já existe uma instância do serviço rodando") |
| `SetMicGlobalMute` e SIGTERM logo em seguida | saída 0; 0 nós; disco com mute global, canal novo e o silêncio do GAME; 4 revisões no histórico |

Achados durante a implementação: (1) por padrão o `zbus` deixa uma segunda instância **assumir** o nome de outra — o servidor agora pede o nome sem
`ReplaceExisting` e sem `AllowReplacement` e há teste para isso; (2) o `busctl` lê `-6` e `-inf` como opções (usar `--`); (3) erros do próprio
barramento (`ServiceUnknown`, `NoReply`…) são classificados no cliente como "serviço indisponível", distintos de "comando recusado".

Limites: medidores de nível não passam pelo IPC (spec 6.4); o serviço ainda não faz troca de perfil nem associação de aplicativos; a janela GTK ainda não usa o cliente.

## Prova 14 e 15 — aplicativos: inventário, regras, roteamento e janela

Scripts: `tools/provas/14-roteamento-e2e.sh` (motor sozinho) e `tools/provas/15-aplicativos-ao-vivo.sh` (serviço + motor + D-Bus + janela).
Só fluxos de teste `IaraTeste*` (pw-play, tom baixo) são movidos; os aplicativos do usuário aparecem como Não atribuídos e não são tocados.

| Etapa | Resultado (links reais no PipeWire) |
| --- | --- |
| Sem regras | A, B e C na saída física |
| `AssignApp` A→game, B→media, C→chat | A→`iara.ch.game`, B→`iara.ch.media`; C (`node.dont-move`) fica na saída física, estado "não aceita ser movido" |
| `SetAppSession` A→chat (só esta sessão) | A→`iara.ch.chat`; **o perfil não muda** (regra de A segue `game`) |
| `RemoveChannel media` com destino `game` | regra de B passa a `game` e o fluxo de B acompanha; as demais regras intactas |
| Mudança externa (pw-metadata) em B | o motor **não briga**; estado "não aplicado/em outro canal" |
| Desassociar A (volta a Não atribuídos) | a sobreposição do Iara é removida e A volta ao padrão do sistema |
| Serviço encerra | os fluxos de teste voltam à saída física |

Achados: (1) as propriedades do anúncio no registro não bastam: `node.dont-move` e parte da identidade só aparecem na informação completa do nó, então
cada fluxo tem um listener de `info` (só aplica eventos que trazem propriedades e nunca apaga um campo por omissão); (2) a chave do aplicativo muda de
`name:` para `bin:` quando a identidade completa chega, então rotas e escolhas de sessão são recalculadas a cada relatório; (3) aplicativos em sandbox
expõem `pipewire.access.portal.app_id` (ex.: `app.zen_browser.zen`), melhor identidade que o binário e usada como id do aplicativo.
A decisão de roteamento é pura (`iara-audio::routing`, 6 testes): move uma vez por destino desejado, respeita `dont-move`, só considera "aplicado" com o link real
no destino e não briga com mudanças externas.

Limites: arrastar e soltar e o menu "Mover para…" **não foram exercitados com entrada real** (só compilados e vistos em captura); unassigned só é capturado
quando a saída padrão do sistema for a do Iara (issue #9); aplicativo com identidade só por `name` pode trocar de chave durante a vida do fluxo.

## Prova 16 e 17 — saída padrão do sistema (issue #9)

Scripts: `tools/provas/16-saida-padrao-e2e.sh` (cenário completo contra o PipeWire **real**, com rede de segurança que devolve o padrão original) e
`17-restauracao-repetida.sh` (regressão: instalar, `kill -9`, `--restore-default`, N vezes). O padrão original era `alsa_output.pci-0000_0b_00.4.analog-stereo`.

| Etapa | Resultado |
| --- | --- |
| Primeira execução, sem saída preferida | padrão do sistema → `iara.unassigned`; o padrão anterior vira a saída preferida do perfil; anterior gravado em disco **antes** da troca; aplicativos reais (Zen, Cider) que seguem o padrão passam a sair pelo Iara |
| Aplicativo novo, sem regra | entra por `iara.unassigned` (estado "aplicado") |
| Aplicativo com saída própria (`--target`) | permanece na saída física; estado `outside` ("fora do mixer") |
| `AssignApp` do aplicativo novo | passa ao canal; regra salva |
| `kill -9` no serviço | padrão fica em `iara.unassigned`; registro continua em disco |
| `iara-service --restore-default` | padrão volta ao original, registro apagado, aplicativos voltam à saída física |
| Reinstalar e o usuário escolher o original | `released`; registro apagado; reinstalação não acontece |
| "Desligar mixer" com posse largada | não altera a escolha do usuário |
| "Desligar mixer" com posse | restaura o original; 0 nós `iara.*` |
| Regressão (17) | 6 de 6 |

Defeitos encontrados e corrigidos neste ciclo: (1) a decisão "há saída física pronta" usava a lista de ausentes do relatório de `apply`, que fica velha
(instalaria com o dispositivo ausente, deixando o sistema mudo, ou não instalaria depois de ele voltar) — a lista agora é única (relatório + eventos), com teste;
(2) **intermitente**: ao encerrar o motor logo depois de enviar a saída padrão restaurada, o processo podia sair com a mensagem ainda no buffer da conexão e a
restauração não acontecia (`--restore-default` falhou numa rodada e passou noutra); o encerramento agora faz uma ida-e-volta (`core.sync`) com prazo de 1 s; (3) o
`Deactivate` D-Bus terminava com "Remote peer disconnected" porque o serviço saía antes de a resposta ser entregue — agora há um instante de graça e o cliente trata
"o serviço sumiu" como sucesso.

Limites: ao trocar a saída padrão, aplicativos que seguem o padrão são movidos pelo WirePlumber e podem ter pequenos cortes. No fim das rodadas o Cider (Electron) estava
sem fluxo de áudio aberto; não se estabeleceu se foi pausa natural ou efeito das trocas (issue aberta). Não testado: queda do PipeWire com o Iara como padrão.

## Prova 18 e 19 — troca de perfil sem vazamento (spec 8.4)

Scripts: `tools/provas/18-troca-sem-vazamento.sh` (motor: remoção de canal com aplicativo nele, com controle) e `tools/provas/19-troca-de-perfil-e2e.sh`
(serviço + D-Bus + PipeWire reais; captura da saída padrão desligada; só fluxos de teste `IaraTeste*`). Os links dos fluxos são amostrados a cada ~25 ms.

**18 — remoção diferida.** O motor remove canais obsoletos só depois de os aplicativos saírem deles (prazo máximo de 5 s), em vez de destruí-los no `apply`.
Fluxo com destino explícito no canal removido, janela de 1 s até o redirecionamento: **com** a espera 46 amostras no canal velho e 0 na saída física; **sem**
a espera (controle) 27 amostras na saída física, ou seja, o vazamento é real e a espera o evita. (Para fluxos movidos por metadata e redirecionados em 50 ms o
controle não vaza: o teste só discrimina com a janela maior.)

**19 — troca de perfil.** Perfil `padrão` (A→aux, B→game) e `copia` (sem aux; A→game). Duplicar, trocar, remover canal na cópia, voltar, e depois 4 trocas seguidas.

| Verificação | Resultado |
| --- | --- |
| Canais no grafo após cada troca | acompanham o perfil ativo (aux some e volta) |
| Fluxos na saída física durante 4 trocas (~725 amostras) | **0** (3 rodadas); antes da correção abaixo, 4 de 4 rodadas tinham de 2 a 6 amostras |
| Trocar para perfil inexistente / ilegível | erro com motivo; perfil, estado, rotas e gravações **intactos** |
| Excluir o perfil ativo | recusado; o último perfil também; excluir outro vai para a lixeira (recuperável) |
| Perfil ativo | persistido em `config.toml` |

Defeito encontrado e corrigido: ~35–70 ms de vazamento ao mover um fluxo para um canal **recém-criado**: o motor tratava o destino como pronto assim que o nó aparecia no
registro, mas o WirePlumber ainda não conseguia ligá-lo (as **portas de entrada** aparecem depois) e usava a saída padrão. O destino agora só vale quando tem ao menos uma porta de entrada.

Regras da troca (serviço): valida o destino antes de mexer em qualquer coisa; grava o perfil atual e fecha o gesto de histórico dele antes; herda os dispositivos do perfil
anterior se o novo não tiver os seus (senão o sistema ficaria mudo); encerra as escolhas só desta sessão; só então aplica o plano e reenvia as rotas. Criar não troca de perfil;
duplicar o ativo copia o estado atual, inclusive ajustes ainda não gravados.

Limites: o menu de perfis da janela foi compilado e visto na captura do cabeçalho, mas o popover e os cliques não foram exercitados com entrada real; importar/exportar
perfis e restaurar revisões ainda não têm comando no IPC nem interface.

## Prova 20 — a janela com entrada real

Script: `tools/provas/20-janela-entrada-real.sh` (auxiliar de acessibilidade em `tools/janela/at.py`). Sobe um serviço isolado e a janela via XWayland
(`GDK_BACKEND=x11`), e a dirige com **mouse e teclado reais** (`xdotool`) e pela **acessibilidade** (AT-SPI, o caminho de um leitor de tela). Cada verificação lê o efeito
**no serviço**, não a aparência. A partir de um ambiente limpo: **25 verificações, 25 ok**.

Defeitos que só apareceram com entrada real (todos corrigidos):

| Defeito | Efeito | Correção |
| --- | --- | --- |
| O botão da etiqueta reivindicava o gesto ao ser pressionado | **arrastar e soltar era impossível** | `DragSource` na fase de captura (o clique simples segue abrindo o menu) |
| O realce "soltar aqui" não saía depois da soltura (dois handlers de `drop`; o primeiro devolve `true`) | coluna ficava marcada | o realce sai dentro do próprio handler |
| Passos de teclado dos sliders de 0,06 dB | ajuste fino por teclado inviável | 1 dB por seta, 6 dB por Page |
| Todos os sliders e botões de mute com nomes acessíveis repetidos ("Volume — ESCUTA") | leitor de tela não distingue os canais | nomes com o canal ("GAME — volume da escuta", "MIC — volume para aplicativos") |
| Campos de texto sem nome acessível | leitor de tela os anuncia vazios | nomes ("Nome do novo canal", "Novo nome do canal") |
| O menu de perfis não fechava ao escolher um perfil | clique seguinte no cabeçalho o fechava | fecha ao escolher |
| **Confirmação de exclusão sobrevivia a fechar e reabrir o menu**; nome acessível não dizia o perfil | um clique casual depois excluía sem nova confirmação | confirmação desfeita ao fechar o menu; nomes "Excluir o perfil X" / "Confirmar a exclusão do perfil X"; estado armado por ícone, cor e nome |
| O interruptor "Mover só nesta sessão" não mostrava que estava ligado | usuário sem como saber o modo | estilo de ligado no cabeçalho |

Confirmado funcionando: arrastar a etiqueta entre colunas salva a regra; com "só nesta sessão" ligado vale só até o app parar e a regra salva não muda; clique simples abre o menu
"Mover para…"; "Reaplicar regra do perfil"; arrastar slider, mute, participação, mute global do MIC, ChatMix (arrasto e "Centro"); criar, trocar e excluir perfil (duas etapas, lixeira);
criar, renomear e remover canal (duas etapas).

Cuidados e limites do método: o `xdotool` move o **ponteiro real** do usuário; as posições da janela mudam entre execuções, então o ponteiro só clica depois de confirmar que
está sobre a janela do Iara (uma versão inicial do roteiro, sem essa trava, deu cliques em coordenadas erradas quando a janela se moveu). A janela foi exercitada via **XWayland**, não no
backend Wayland nativo; popovers foram acionados pela acessibilidade (sem posições no backend X11); Tab/Enter puro para chegar a uma etiqueta e abri-la, e um leitor de tela real (Orca),
não foram testados.

### O que isto NÃO prova

- Medições longas (horas), CPU com medidores de nível ativos e com efeitos; o tempo de indisponibilidade após reinício do PipeWire; clientes de captura reais (OBS/Discord) lendo a fonte virtual; saída sem hot-plug físico real (o perfil de placa foi desligado por software); Bluetooth.
- Microfone físico real (a prova 02 usa um sink nulo como fonte simulada).
- Provas 01–02 usaram `pw-loopback`/`pw-cli`; a 03 usa um processo Rust, mas só no contexto cliente. Hospedagem no daemon (módulo em `pipewire.conf.d`) não foi testada e a decisão de hospedagem segue aberta.
- A diferença de ~0,2 dB entre o RMS esperado (−9,03) e o medido (−9,25) na base não foi investigada.

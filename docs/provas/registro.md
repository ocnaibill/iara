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

### O que isto NÃO prova

- Mute por flag (apenas ganho 0 foi testado); MASTER após a soma de dois canais; ausência de cópias duplicadas com mais de um canal (passo 3).
- Passos 4–8: Não atribuídos fora da transmissão, MIC dedicado, reconexão, hospedagem/queda do processo, mudança de aplicativos, CPU/RAM/xruns.
- Os nós foram hospedados por processos `pw-loopback`/`pw-cli`, não por um serviço Rust; a decisão de hospedagem segue aberta.
- A diferença de ~0,2 dB entre o RMS esperado (−9,03) e o medido (−9,25) na base não foi investigada.

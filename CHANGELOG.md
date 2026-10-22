# Changelog

Todas as mudanças notáveis deste projeto serão documentadas neste arquivo.

O formato é baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/)
e este projeto adere ao [Versionamento Semântico](https://semver.org/lang/pt-BR/).

## [0.1.0] - 2026-10-22

### Adicionado
- **Curva natural de fader (Quadratic Taper)**:
  - Mapeamento perceptivo natural dos sliders de ganho: $dB = -60 \cdot (1 - p)^2$.
  - 50% no slider corresponde a −15 dB (área útil de ajuste musical e conversação); 75% corresponde a −3,75 dB.
  - Posição 0 representa silêncio absoluto (−∞ dB).
- **Controle e sincronização de volume de hardware**:
  - Integração bidirecional do fone/saída física com WirePlumber (`wpctl`).
  - Slider e indicador de volume físico na engrenagem de saída Master da interface.
  - Sincronização automática com teclas multimídia do teclado e painéis de som do desktop (ajustes no dispositivo padrão refletem instantaneamente no Iara).
  - Prevenção do problema de atrito onde o WirePlumber inicializava saídas físicas em −24 dB (0,40 linear).
- **Identificação persistente de dispositivos USB (Issue #8)**:
  - Algoritmo de resolução inteligente tolerante a mudanças de porta USB, índice de barramento e identificadores voláteis de hardware.
  - Preserva preferências de fone e microfone entre reinicializações e trocas de conexão física.
- **Ocultação de barramentos internos no desktop (Issue #17)**:
  - Barramentos virtuais internos (`Mix`, `Master`, `MicInput`, `MicCommon`, `MicApps`) agora utilizam `media.class = Audio/Sink/Internal`.
  - O painel de som do desktop e o `pipewire-pulse` exibem apenas a saída de captura do Iara (`iara.unassigned`), evitando que o sistema jogue áudio acidentalmente em nós de mixagem internos.
- **Empacotamento e Integração de Desktop**:
  - Arquivo de inicialização de desktop XDG (`dev.iara.Mixer.desktop`).
  - Metadados AppStream 0.16 (`dev.iara.Mixer.metainfo.xml`).
  - Ícone vetorial SVG em `hicolor` (`dev.iara.Mixer.svg`).
  - Unidade de serviço systemd de usuário (`systemd/user/iara.service`).
  - Makefile de compilação e instalação (`make install`).

### Corrigido
- Ajuste de sensibilidade dos medidores de pico (RMS/Peak) com piso em −60 dB compatível com a nova escala.
- Serialização segura de tipos numéricos no D-Bus para clientes sem suporte nativo a `Option<f64>`.
- Eliminação de alertas de linter (Clippy) e garantia de conformidade estrita em todo o workspace.

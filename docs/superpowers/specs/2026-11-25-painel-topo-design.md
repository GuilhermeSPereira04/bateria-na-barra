# Painel do topo — design

Data: 2026-11-25
Projeto: `D:\dev\battery-bar` (o painel entra neste mesmo programa)

## Intenção

Um **painel que desce da borda de cima da tela** no Windows 11 e substitui, no
uso do dia a dia, dois menus do sistema: o **Iniciar** (achar e abrir apps) e
as **configurações rápidas** (Win+A: rede, Bluetooth, som). Ele também mostra
a bateria dos periféricos, o consumo do PC e um calendário simples.

Inspiração: o vídeo `D:\download\SaveClip.App_AQN_J_84GkG5...mp4`, um Reel do
programa Hitodama (hitodama.in). Lá, um painel com abas (Controls, Today,
Stats, Clipboard, Apps, Wallpaper, Power) desce do topo da tela.

**Por quê:** o usuário quer o próprio painel, do jeito dele e **muito leve**,
porque vai ficar rodando o tempo todo. Por isso é em Rust, sem tecnologia web.
Esse painel serve de referência pro *visual*, mas não pra *tecnologia* (o
Hitodama e programas parecidos costumam carregar um navegador por baixo).

## Objetivos

- Abrir o painel ao **segurar o mouse 1,5 s parado na borda de cima** da tela.
- Ter à vista, sempre que abrir: busca de apps, botões de rede/Bluetooth/áudio,
  bateria dos periféricos e o player de música.
- Mostrar em abas pequenas: recursos (CPU, GPU, RAM, processos que mais
  gastam), calendário e speed test.
- Aparência de **vidro fosco do Windows 11**, com cores que o usuário pode
  trocar.
- Continuar leve: com o painel fechado, o programa todo (faixa + painel) gasta
  praticamente o mesmo que o battery-bar gasta hoje.

## Fora do escopo (agora)

- **Troca de wallpaper com o Wallpaper Engine:** fica pra **fase 2** (ver o
  fim do documento). O desenho não deve impedir uma aba nova depois.
- Clipboard, clima, pomodoro, botões de energia (desligar/reiniciar), agenda
  com eventos no calendário.
- "Liquid glass" de verdade, com a borda distorcendo o fundo como lente. Isso
  exigiria capturar a tela atrás do painel e processar na GPU sem parar.
- Abrir pela tecla Windows sozinha, ou substituir o Iniciar a nível de sistema.
  O Iniciar original continua lá; o painel só passa a ser o caminho do usuário.
- Segundo monitor. O painel só funciona no monitor principal.

## Anti-objetivos (falhou, mesmo funcionando)

- **Pesar.** Gastar CPU ou RAM perceptíveis com o painel fechado, ou ficar
  medindo recursos em segundo plano.
- **Abrir sem querer.** Se o painel descer quando o usuário só vai clicar numa
  aba do navegador ou no X de uma janela maximizada, o projeto falhou.
- **Atrapalhar jogo/vídeo em tela cheia:** abrir por cima de um jogo, roubar
  o foco dele ou fazer ele perder quadros.
- **Quebrar o battery-bar.** A faixa da barra de tarefas e as leituras de
  bateria continuam funcionando exatamente como hoje.

## Restrições

- Windows 11, Rust (edition 2024), crate `windows` (como já é hoje).
- **Sem webview nem framework de interface.** A interface é desenhada à mão
  com Win32 + Direct2D/DirectWrite, igual à faixa atual.
- Um programa só (`battery-bar.exe`), uma instância. O mutex atual continua
  valendo.
- Metas de consumo (medidas na coluna **"Memória"** do Gerenciador de
  Tarefas, que é o conjunto de trabalho privado, ou seja, o número que o
  usuário vê; *decisão minha*):
  - **Painel fechado:** CPU ~0% e memória até ~8 MB (hoje está em ~4 MB).
    Vale também **depois** de abrir e fechar o painel. Ao fechar, libere os
    recursos de desenho e apare o conjunto de trabalho (`EmptyWorkingSet` /
    `SetProcessWorkingSetSize(-1,-1)`). Não importa se as DLLs carregadas
    (D3D, WinRT, Core Audio, PDH, WinHTTP) continuam na memória confirmada.
    Tudo num processo só; nada de processo filho.
  - **Painel aberto:** CPU baixa, só com a atualização de recursos a cada 1 s;
    memória até ~20 MB. Ao fechar, a memória extra é liberada (ver
    "Deixado pro construtor").
- Dependências novas só se forem pequenas e não houver jeito razoável com a
  crate `windows`. Não pode entrar runtime async (tokio) nem cliente HTTP
  pesado.

## Abordagens consideradas

1. **Crescer o battery-bar, com interface desenhada à mão (Win32 + Direct2D).
   ESCOLHIDA.**
   O resultado é o mais leve possível, e a janela especial, a detecção de tela
   cheia e as leituras de bateria são reaproveitadas. O custo é fazer cada peça
   de interface à mão: campo de busca, abas, hover e animação.
2. **Projeto novo com Slint.** Os controles já vêm prontos, gastando uns
   15–30 MB. Encaixar o vidro do Windows e a janela sem borda colada no topo
   dá trabalho, e o resultado seria dois programas, ou então seria preciso
   migrar o battery-bar pra dentro dele.
3. **Projeto novo com egui.** É o mais rápido pra ter algo rodando, mas gasta
   40 MB ou mais, a transparência funciona mal e a cara é de ferramenta de
   programador.
4. *Descartada logo:* Tauri/Electron/WebView2, porque carregam um navegador
   (100 MB ou mais) e vão contra o motivo do projeto.

O usuário escolheu a **1**, porque leveza é o motivo do projeto e o battery-bar
já resolveu as partes difíceis.

## Como o código está hoje (base)

- `app.rs`: janela dona invisível. Guarda o estado (`App`) e trata
  mensagens: leituras, timers, cliques na faixa/bandeja, troca de tema e
  reinício do Explorer.
- `readers/`: uma thread por dispositivo (mouse Zaopin, DualSense, celular via
  ADB). Cada uma manda `(DeviceKind, ReadOutcome)` por canal e acorda a UI com
  `WM_READING`. `model::Tracker` guarda o último valor, o tempo de validade
  etc.
- `render.rs`: desenha a faixa com Direct2D/DirectWrite num bitmap, que vai
  pra tela via `UpdateLayeredWindow`.
- `taskbar.rs`: janela da faixa (`WS_EX_LAYERED | TOPMOST | NOACTIVATE`),
  posicionamento e **detecção de tela cheia** (`fullscreen_active`), que o
  painel deve reaproveitar.
- `tray.rs`, `startup.rs`, `theme.rs`: ícone da bandeja e menu, iniciar com o
  Windows, tema claro/escuro.

**Atenção:** a faixa usa janela *layered* com `UpdateLayeredWindow`, e esse
tipo de janela **não recebe o vidro do Windows**. O painel precisa ser uma
janela diferente: popup sem borda, com o fundo do sistema ligado via
`DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW)`, um
frame estendido pro cliente e Direct2D desenhando por cima com fundo
transparente. Cantos arredondados via `DWMWA_WINDOW_CORNER_PREFERENCE`.

## Comportamento

### Abrir e fechar

- **Abre** quando o cursor fica **1,5 s** na linha de cima (y = topo) do
  **monitor principal**, em qualquer ponto da largura. Se o cursor sair da
  borda antes disso, o contador zera. Com o mouse na borda, não precisa ficar
  imóvel ao pixel: basta continuar na borda.
- **Não abre** se `fullscreen_active` disser que há jogo, vídeo ou
  apresentação em tela cheia, nem se o painel já estiver aberto.
- **Fecha** quando:
  - o cursor sai do painel e fica fora por ~0,5 s (uma folga pra não fechar
    por um escorregão). *Decisão minha:* essa regra **fica pausada**
    enquanto há texto na busca, um speed test rodando, ou a ⚙ / a lista de
    saídas de áudio aberta. Também só começa a valer **depois que o cursor
    entrou no painel pela primeira vez**, porque quem abre pela bandeja tem o
    cursor lá embaixo. Esc e clique fora continuam fechando sempre;
  - o usuário aperta **Esc**;
  - o usuário clica fora;
  - o usuário abre um app pela busca.
- **Ao abrir, o painel pega o foco do teclado**, pra que digitar vá direto pra
  busca (diferente da faixa, que nunca pega foco). Ao fechar, o foco volta pra
  janela que estava ativa antes.
  *Decisão minha:* o Windows pode recusar o foco, porque quem abriu foi o
  mouse parado e não um clique. Pra contornar, use `AttachThreadInput` com a
  thread da janela ativa + `SetForegroundWindow`, e **nunca injete tecla
  falsa** (`SendInput` de Alt), porque isso vazaria pra janela anterior, por
  exemplo um jogo em janela. Se mesmo assim o foco não vier, o painel
  funciona normalmente e **um clique** em qualquer lugar dele (ou na busca)
  pega o foco.
- Animação de descer/subir curta (~150 ms) ou nenhuma. Ver "Deixado pro
  construtor".
- O painel fica centralizado no topo do monitor principal, colado na borda,
  com cantos de baixo arredondados.

### Layout (misto)

```
╭──────────────── borda de cima da tela ─────────────────╮
│ 🔍 Pesquisar apps…                                  ⚙  │
│ [Wi‑Fi] [BT] [🔊 60%]   🖱 25%  🎮 95%  📱 81%⚡       │
│ ♪ Kick Back · Kenshi Yonezu        ⏮  ⏯  ⏭            │
│ Recursos | Calendário | Speed test                     │
│ CPU 23% · GPU 41% · RAM 70%                            │
│ Chrome 2,1 GB · VS Code 1,4 GB · Discord 600 MB        │
╰────────────────────────────────────────────────────────╯
```

Parte fixa, sempre visível: busca, botões rápidos, bateria e player. Parte de
baixo: abas pequenas, e a primeira aberta é **Recursos**. Quando há texto na
busca, a parte de baixo vira a **lista de resultados**; ao apagar o texto,
volta pras abas. Mockups de referência em
`.superpowers/brainstorm/5363-1795573161/content/` (`layout.html` opção C,
`vidro.html` opção A).

### Busca de apps

- Lista os apps do **menu Iniciar**, tanto os Win32 quanto os da Microsoft
  Store. A fonte única é a pasta virtual `shell:AppsFolder` (`FOLDERID_AppsFolder`),
  e cada item é aberto com `shell:AppsFolder\<AppUserModelID>`.
- Começar a digitar já filtra (não precisa clicar na busca). O filtro casa
  sem diferença de maiúsculas/acentos e põe primeiro quem *começa* com o
  texto, depois quem *contém*.
- Mostra até ~8 resultados, com ícone e nome. ↑/↓ escolhem, Enter abre o
  selecionado (o primeiro, por padrão), e clique também abre.
- A lista de apps pode ser montada ao abrir o painel ou guardada em cache. Ver
  "Deixado pro construtor".

### Botões rápidos

- **Wi‑Fi:** mostra ligado/desligado e o nome da rede. Clique liga/desliga o
  rádio (`Windows.Devices.Radios`). Uma setinha ou clique direito abre
  `ms-settings:network-wifi`.
- **Bluetooth:** liga/desliga o rádio e mostra quantos aparelhos estão
  conectados. Setinha ou clique direito abre `ms-settings:bluetooth`.
- **Áudio:** mostra o volume e o nome da saída atual. Rolar a roda do mouse
  em cima muda o volume (Core Audio, `IAudioEndpointVolume`), e clique liga/
  desliga o mudo. A setinha abre uma lista com as saídas pra **trocar o
  dispositivo de saída**. Isso usa a interface não documentada `IPolicyConfig`,
  a mesma que o EarTrumpet e ferramentas parecidas usam; se ela falhar, a
  setinha abre `ms-settings:sound`.

### Bateria

- Mostra os mesmos três dispositivos e estados da faixa (%, carregando, baixa,
  `--%`), lendo do **mesmo `Tracker`**. Não cria leitores novos nem aumenta a
  frequência de leitura.
- Abrir o painel **não** dispara leitura extra. As threads continuam no ritmo
  de hoje.

### Player

- Mostra a música que está tocando (título, artista e capa se houver) com
  ⏮ ⏯ ⏭, via `GlobalSystemMediaTransportControlsSessionManager` (WinRT). Isso
  funciona com Spotify, navegadores e qualquer app que avise o Windows.
- Sem nada tocando, a linha mostra "Nada tocando" esmaecido (ou some; quem
  decide é o construtor).

### Aba Recursos

- **CPU %** (total), **GPU %** (maior uso entre os motores da GPU, via contador
  PDH `\GPU Engine(*)\Utilization Percentage`) e **RAM** (usada/total e %).
- **Processos que mais gastam:** os 5 primeiros, mostrando nome, CPU % e RAM.
  Por padrão a lista é ordenada por **CPU**, e clicar no título da coluna RAM
  ordena por RAM. Processos com o mesmo nome (ex.: várias abas do Chrome)
  aparecem **somados** numa linha, como no Gerenciador de Tarefas.
- Atualiza **a cada 1 s, só com o painel aberto e na aba Recursos**. Fechado,
  nada é medido.

### Aba Calendário

- Mês atual em grade (dom–sáb), com o dia de hoje destacado e setas pra ir pro
  mês anterior/seguinte. Não tem eventos nem integração com nenhuma agenda.

### Aba Speed test

- Botão "Testar". Só roda quando o usuário clica, nunca sozinho.
- Mede **ping, download e upload** contra os endpoints públicos da Cloudflare
  (`speed.cloudflare.com/__down?bytes=…` e `/__up`) via **WinHTTP**, sem crate
  HTTP. Roda numa thread separada e mostra o progresso enquanto mede. Fechar o
  painel no meio cancela o teste.
- Guarda o último resultado e a hora, enquanto o programa estiver aberto.

### Cores e aparência

- O fundo é o **vidro fosco do Windows 11** (backdrop do sistema), mais escuro
  e discreto (opção A do mockup `vidro.html`).
- Dá pra mudar duas coisas:
  - **Cor de destaque:** botões ligados, barras de CPU/RAM, aba ativa e dia de
    hoje. Pode ser uma de ~8 cores prontas ou **"usar a cor do Windows"** (cor
    de destaque do sistema, que acompanha se o usuário mudar no Windows).
  - **Tom do vidro:** escuro, claro, ou levemente puxado pra cor de destaque.
    É uma camada de cor semitransparente desenhada por cima do vidro.
- Onde escolher: a **engrenagem ⚙** no canto do painel abre, dentro do próprio
  painel, a fileira de cores e os tons. A mudança aparece na hora.
- A escolha é salva em `%APPDATA%\battery-bar\config` (formato à escolha do
  construtor, editável à mão; ver abaixo). Quem quiser uma cor exata pode pôr
  um hex lá.
- A faixa da barra de tarefas **não** muda de cor. Ela continua seguindo o
  tema claro/escuro da barra.

### Bandeja e inicialização

- O menu da bandeja e o "Iniciar com o Windows" continuam como hoje.
  *Decisão minha:* o menu ganha o item **"Abrir painel"**, pra ter um jeito
  de abrir sem passar pela borda.

## Decisões tomadas (com o usuário)

- Painel próprio, em Rust, focado em leveza. Não instalar o Hitodama.
- Recursos: rede, Bluetooth, áudio, player, bateria dos periféricos,
  CPU/GPU/RAM + processos, speed test, busca de apps, calendário simples.
  Wallpaper Engine fica pra fase 2.
- Abre segurando o mouse **1,5 s** na borda de cima.
- Layout **misto**.
- Aparência **vidro fosco do Windows**, com cores trocáveis.
- Construído **dentro do battery-bar**, desenhado à mão (abordagem 1).

## Decisões minhas (o usuário aprovou o resumo, mas vale conferir)

- Fecha ao sair com o mouse (folga de 0,5 s), com Esc ou com clique fora.
  A regra do mouse fica pausada durante busca, speed test, ⚙ ou lista de
  áudio, e só começa a valer depois que o cursor entra no painel.
- Foco via `AttachThreadInput`, sem tecla falsa. Se falhar, um clique pega o
  foco.
- Meta de memória medida na coluna "Memória" do Gerenciador de Tarefas,
  inclusive depois de abrir/fechar. Tudo num processo só.
- Não abre em tela cheia. Só no monitor principal.
- Recursos atualizam a cada 1 s e só com o painel aberto. O speed test só
  roda com clique.
- Busca via `shell:AppsFolder`. Enter abre o primeiro resultado.
- Troca de saída de áudio via `IPolicyConfig`, caindo pra `ms-settings:sound`
  se falhar.
- Processos ordenados por CPU e somados por nome.
- Speed test pela Cloudflare via WinHTTP.
- Cores: ~8 prontas + cor do Windows + tom do vidro, escolhidas na ⚙ e
  salvas em `%APPDATA%\battery-bar\config`.
- O repositório e o `.exe` continuam com o nome `battery-bar` por enquanto.
- O menu da bandeja ganha "Abrir painel".

## Deixado pro construtor

- Detectar a borda com hook de mouse de baixo nível ou com consulta periódica
  do cursor. O que importa é que custe ~0% de CPU e funcione por cima de
  qualquer janela, inclusive maximizada.
- Desenhar o painel: `ID2D1HwndRenderTarget`, swap chain DXGI com
  DirectComposition ou o que funcionar melhor com o backdrop do DWM.
  Precisa ser compatível com o vidro do sistema e não pode gastar quando
  fechado.
- Se o painel é destruído ao fechar ou só escondido (com recursos de desenho
  liberados), desde que a meta de memória fechado seja cumprida.
- Animação de abrir (curta ou nenhuma).
- Tamanhos, fontes, espaçamentos e ícones (sugestão: Segoe Fluent Icons, que
  a faixa já usa pra glifos).
- Formato do arquivo de config (ex.: TOML ou `chave=valor` simples) e se vale
  uma crate pequena pra isso.
- Cache e atualização da lista de apps, e como pegar os ícones
  (`IShellItemImageFactory`).
- Organização dos módulos novos (ex.: `panel/`, `sysinfo.rs`, `media.rs`,
  `apps.rs`, `speedtest.rs`).
- Testes: seguir o estilo do projeto (lógica pura testada, como
  `compute_strip_rect` e `fullscreen_active`). Exemplos de lógica pura daqui:
  o contador de 1,5 s da borda, o filtro da busca, a soma de processos e a
  grade do calendário.

## Fase 2 (fora deste trabalho): Wallpaper Engine

Uma aba "Wallpaper" listaria os wallpapers inscritos do usuário (pasta do
workshop da Steam, app 431960) com miniatura, e o clique aplicaria via linha
de comando do Wallpaper Engine
(`wallpaper32.exe -control openWallpaper -file <project.json>`). Fica aqui só
pra lembrar que a aba nova precisa caber no layout.

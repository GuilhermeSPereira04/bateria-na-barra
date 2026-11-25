# Painel do topo — ajustes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Abrir em 0,5 s, vidro acrylic com transparência ajustável na ⚙, lista suspensa de áudio com barra de volume, e memória do painel aberto ≤ ~20 MB.

**Architecture:** Mudanças locais no painel existente (`src/panel/*`, `src/app.rs`, `src/config.rs`, `src/audio.rs`). Um controle de "barra" genérico (`Hit::Slider`) com arrasto via captura do mouse, usado pela transparência e pelo volume. Um argumento de linha de comando `--painel abrir|fechar` repassado à instância que já está rodando, que serve pra medir memória sem mexer no mouse.

**Tech Stack:** Rust 2024, crate `windows` 0.62 + `windows-core`, Direct2D, DWM, `SetWindowCompositionAttribute` (user32, carregada dinamicamente).

**Spec:** `docs/superpowers/specs/2026-11-25-painel-topo-ajustes-design.md`. Ela se apoia em `docs/superpowers/specs/2026-11-25-painel-topo-design.md`. Leia as duas.

## Global Constraints

- Repositório `D:\dev\battery-bar`, branch `feat/painel-topo`. Em bash, cargo precisa de `export PATH=/d/mingw64/bin:$PATH` pra linkar.
- Memória (coluna "Memória" do Gerenciador = contador `\Process(battery-bar)\Working Set - Private`): painel fechado ≤ ~8 MB, inclusive depois de abrir/fechar; aberto ≤ ~20 MB.
- O vidro tem que continuar **transparente com desfoque** depois de qualquer mudança no jeito de desenhar. Fundo preto ou opaco é falha.
- **Pode** rodar `target\release\battery-bar.exe` (o usuário autorizou): feche antes a instância dele (`Stop-Process -Name battery-bar`). **Nunca** mova o mouse nem digite. Pra abrir/fechar o painel use `--painel abrir|fechar` (Task 1). Prints da tela com PowerShell `CopyFromScreen` são permitidos. Ao terminar cada task, **deixe `target\release\battery-bar.exe` rodando**.
- Nunca ligue/desligue rádios, mude o áudio, mande comando de mídia nem rode o speed test.
- Sem `unwrap`/panic em chamadas de sistema (release usa `panic = "abort"`). Finais de linha LF. Confira se o `git diff --stat` está proporcional à mudança.
- IDs de mensagem em uso: `WM_APP+1..+5`, `+10` (ASYNC), `+11` (CLOSED), `+12` (HIT), mais as de tecla e roda que já existem (confira em `src/panel/mod.rs`). Os novos ficam em `WM_APP+20` em diante.
- Commits em inglês `feat:`/`fix:`, terminando com `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Arrastar uma barra e sair com o mouse do painel** → o painel não fecha durante o arrasto. Teste `drag_pauses_close` (Task 3).
2. **`transparency` fora do intervalo ou inválido na config** → limita a 0..=100, e lixo vira o padrão 45. Teste `transparency_clamps_and_defaults` (Task 3).
3. **Transparência 0% e 100%** → alpha `0xFF` e `0x00` no gradiente, sem estouro. Teste `gradient_extremes` (Task 3).
4. **PC com mais de 8 saídas de áudio, ou nenhuma** → a lista mostra no máximo 8, e com 0 só a barra. Teste `audio_menu_caps_outputs` (Task 4).
5. **`--painel abrir` sem instância rodando** → o programa inicia normal, sem travar nem sair. Teste `parse_panel_arg` (Task 1). O comportamento sem instância fica no checklist.

---

### Task 1: Abrir em 0,5 s e `--painel abrir|fechar`

**Files:** Modify `src/panel/trigger.rs`, `src/main.rs`, `src/app.rs`, `README.md`.

**Interfaces:**
- Produces:
  - `HOLD_TO_OPEN = Duration::from_millis(500)`.
  - `pub fn parse_panel_arg(args: &[String]) -> Option<bool>` em `main.rs`: `["--painel","abrir"]` → `Some(true)`, `["--painel","fechar"]` → `Some(false)`, qualquer outra coisa → `None`.
  - `pub const WM_PANEL_REMOTE: u32 = WM_APP + 20;` em `app.rs`, com `wParam` 1 = abrir e 0 = fechar. O dono trata como o item "Abrir painel" do menu (sem checar a borda) ou como `close_panel()`.
  - Em `main`: se o mutex já existe e há argumento, faz `FindWindowW("BatteryBarOwner")` e `PostMessageW(WM_PANEL_REMOTE)` e sai. Se não há outra instância, ignora o argumento e inicia normal.

- [ ] **Step 1: Testes.** Em `trigger.rs`, ajuste os testes atuais aos 500 ms: os nomes ficam, os tempos mudam (ex.: `fires_after_hold` com 499 ms → false e 500 ms → true). Em `main.rs`, `parse_panel_arg` com os 3 casos acima mais `[]` e `["--painel"]` → `None`.
- [ ] **Step 2:** `cargo test` → os testes novos e os ajustados falham.
- [ ] **Step 3:** Implementar e trocar no README "1,5 s" por "0,5 s". Acrescentar uma linha: `battery-bar.exe --painel abrir` abre o painel na instância que está rodando (útil pra atalho).
- [ ] **Step 4:** `cargo test` passa e `cargo build --release` não dá avisos. Rodar o exe (Global Constraints), depois `battery-bar.exe --painel abrir` → um print mostra o painel. `--painel fechar` → some.
- [ ] **Step 5:** Commit `feat: open the panel after 0.5 s and accept --painel abrir|fechar`.

---

### Task 2: Memória do painel aberto

**Files:** Create `tools/medir-memoria.ps1`. Modify `src/panel/mod.rs` (e o que a medição apontar).

**Interfaces:**
- Consumes: `--painel abrir|fechar` (Task 1).
- Produces: `tools/medir-memoria.ps1`, um script PowerShell que:
  1. mede `Working Set - Private` (MB, 1 casa) do `battery-bar` fechado;
  2. roda `--painel abrir`, espera 3 s e mede aberto (aba Recursos);
  3. faz 10 ciclos abrir (2 s) / fechar (1 s);
  4. espera 3 s e mede fechado de novo.
  
  Imprime `fechado-inicio`, `aberto`, `fechado-apos-10` (a Task 5 reusa).

- [ ] **Step 1:** Escrever o script e medir a versão atual. Anotar os 3 números no relatório. Referência: o usuário viu 70 MB aberto.
- [ ] **Step 2:** Descobrir o que pesa. Hipótese principal: `create_target` em `panel/mod.rs:216` usa `D2D1_RENDER_TARGET_TYPE_DEFAULT` (hardware → D3D + driver da GPU). Teste a hipótese trocando só isso e medindo de novo. Mostre os números de antes e depois.
- [ ] **Step 3:** Aplicar a correção que fizer o painel aberto ficar ≤ ~20 MB e o fechado ≤ ~8 MB, **mantendo o vidro transparente** (Global Constraints). Atenção: um render target de software numa HWND apresenta via GDI e pode perder o alpha, deixando o fundo preto. Se isso acontecer, use outro caminho, por exemplo um DIB premultiplicado de 32 bpp desenhado com `ID2D1DCRenderTarget` e apresentado de forma que o DWM respeite o alpha, ou D3D11 WARP + DirectComposition. Confirme com um print sobre o papel de parede. Os 136 testes continuam passando.
- [ ] **Step 4:** Rodar `tools/medir-memoria.ps1` e colocar os números no relatório e na mensagem do commit.
- [ ] **Step 5:** Commit `fix: keep the open panel light (<20 MB)`, com os números no corpo da mensagem.

---

### Task 3: Vidro acrylic e barra de transparência

**Files:** Create `src/panel/glass.rs`. Modify `src/config.rs`, `src/panel/{mod,view,layout,draw,trigger}.rs`, `src/app.rs`.

**Interfaces:**
- Produces:
  - `config`:
    - `Config { accent, tint, transparency: u8 }`, padrão `45`. Linha `transparency=N` no arquivo.
    - `Colors` ganha `transparency: u8`.
    - `resolve` → `tint: (u32, f32)`, onde a cor é escuro `0x20242C`, claro `0xF3F3F3` ou a cor de destaque, e `alpha = (100 - transparency) / 100`.
  - `glass.rs`:
    - `pub fn gradient_abgr(rgb: u32, alpha: f32) -> u32` (puro). O alpha vai limitado a 0..=1 e arredondado pra byte.
    - `pub fn apply(hwnd: HWND, rgb: u32, alpha: f32) -> bool`. Carrega `SetWindowCompositionAttribute` de `user32.dll` via `GetProcAddress` e manda `WCA_ACCENT_POLICY` (19) com `ACCENT_ENABLE_ACRYLICBLURBEHIND` (4) e `GradientColor = gradient_abgr(..)`. Em sucesso, desliga o backdrop (`DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_NONE`). Se falhar, devolve `false` e o chamador mantém `DWMSBT_TRANSIENTWINDOW` (o comportamento de hoje).
  - `view.rs`:
    - `pub enum SliderKind { Transparency, Volume }`.
    - `Hit::Slider(SliderKind)` substitui nada; é uma variante nova.
  - `layout.rs`: `pub fn slider_value(track: D2D_RECT_F, x: f32) -> u8` (0 na esquerda, 100 na direita, limitado). Com `gear_open`, aparece uma **segunda** faixa de 40 DIP com o rótulo `Transparência`, a barra (`Hit::Slider(Transparency)`) e o valor `NN%`. A ⚙ passa a somar 80 DIP no total.
  - `panel/mod.rs`, arrasto:
    - `WM_LBUTTONDOWN` em cima de um `Hit::Slider(k)` → `SetCapture`, guarda `k`, põe `(k, valor, false)` numa fila do `Shared` e posta `WM_PANEL_DRAG = WM_APP + 21`.
    - `WM_MOUSEMOVE` com captura faz o mesmo.
    - `WM_LBUTTONUP` com captura → `(k, valor, true)` + `ReleaseCapture`, e **não** gera clique.
    - `pub fn take_drags(&self) -> Vec<(SliderKind, u8, bool)>`.
  - `trigger.rs`: `Pauses` ganha `dragging: bool`, que pausa a `CloseRule`.
  - `app.rs`: `on_drag(kind, value, final)`. Pra `Transparency`: atualiza a config em memória, chama `glass::apply` e redesenha. Só faz `config::save` quando `final`.
  - `draw.rs`: remove o retângulo de tom em tela cheia, porque a cor agora vem do acrylic, e desenha as barras (trilho, parte cheia na cor de destaque, bolinha).
- Antes do acrylic ser aplicado (ou no fallback), o desenho precisa continuar correto.

- [ ] **Step 1: Testes:**
  - `config`:
    - `transparency_roundtrip` (serialize/parse com 0, 45 e 100).
    - `transparency_clamps_and_defaults`: `transparency=150` → 100, `transparency=abc` → 45, e sem a linha → 45.
    - `alpha_from_transparency`: `resolve` com 45 → alpha 0.55.
  - `glass`:
    - `gradient_extremes`: `gradient_abgr(0x20242C, 1.0) == 0xFF2C2420` e `gradient_abgr(0x20242C, 0.0) == 0x002C2420`.
  - `layout`:
    - `slider_value` dá 0 na borda esquerda, 100 na direita, 50 no meio, e fica limitado fora do trilho.
    - `gear_adds_color_row` passa a esperar +80 DIP e a região `Hit::Slider(Transparency)`.
    - O teste de 144 dpi sem sobreposição continua cobrindo a ⚙ aberta.
  - `trigger`:
    - `drag_pauses_close`: armado, fora há 3 s com `dragging=true` → false. Ao soltar, fecha 0,5 s depois.
- [ ] **Step 2:** `cargo test` → falham.
- [ ] **Step 3:** Implementar. Ao abrir o painel, chame `glass::apply` com as `Colors` resolvidas, e de novo a cada mudança de tom, cor ou transparência.
- [ ] **Step 4:** `cargo test` e build sem avisos. Rodar e abrir com `--painel abrir` → print sobre o papel de parede mostrando desfoque transparente escuro.
  - Edite a config na mão pra `transparency=80`, feche e abra → mais transparente (print).
  - Volte pra 45.
  - Rode `tools/medir-memoria.ps1` → dentro da meta.
- [ ] **Step 5:** Commit `feat: acrylic glass with adjustable transparency`.

---

### Task 4: Lista suspensa de áudio com barra de volume

**Files:** Modify `src/audio.rs`, `src/panel/{view,layout,draw,mod}.rs`, `src/app.rs`.

**Interfaces:**
- Consumes: `Hit::Slider`, `SliderKind::Volume`, `slider_value`, `take_drags`, `Pauses.dragging` (Task 3).
- Produces:
  - `audio`:
    - `AudioSnapshot` ganha `default_id: Option<String>`.
    - `pub fn set_volume(pct: u8)` (via `SetMasterVolumeLevelScalar`).
  - `view`: `pub struct AudioMenu { pub outputs: Vec<String>, pub current: Option<usize>, pub volume: u8 }`. `PanelView.audio_list` é **substituído** por `pub audio_menu: Option<AudioMenu>`.
  - `layout`: `pub const MAX_OUTPUTS: usize = 8`. Com `audio_menu`, a lista suspensa fica alinhada à esquerda do cartão de áudio, logo abaixo dele, com 240 DIP de largura. A primeira linha (36 DIP) é a barra `Hit::Slider(Volume)` com o `NN%`. Depois vêm até 8 linhas de 30 DIP (`Hit::AudioDevice(i)`). As regiões da lista vão **antes** das outras na ordem do `hit_test` (ficam por cima). O corpo do painel **não** muda mais por causa do áudio.
  - `draw`: a lista por último, por cima de tudo, num cartão mais opaco (branco a 12% sobre um fundo escuro a 85%, pra ler por cima do conteúdo). `✓` (glifo Segoe Fluent `U+E73E`) na saída atual.
  - `app`:
    - `AudioMore` abre e fecha a lista.
    - Um clique em qualquer região que não seja da lista, com ela aberta, **só fecha a lista** (o clique é consumido).
    - Esc com a lista aberta fecha só a lista.
    - `on_drag(Volume, v, _)` → `audio::set_volume(v)` e atualiza o estado.
    - `AudioDevice(i)` → `set_default` (com o fallback de hoje) e fecha a lista.
    - A roda do mouse em cima da lista muda o volume, igual à roda no cartão.
    - `Pauses.popup_open` usa `audio_menu.is_some()`.
    - Com o áudio `Unavailable`, `AudioMore` não faz nada.

- [ ] **Step 1: Testes** (`layout`):
  - `audio_menu_overlays_below_audio_card`: o topo da lista é ≥ a base do cartão de áudio. Um ponto numa linha `AudioDevice(0)` que cai em cima da área das abas devolve `AudioDevice(0)`. As abas continuam existindo.
  - `audio_menu_caps_outputs`: com 12 saídas → `AudioDevice(0..8)`, sem `AudioDevice(8)`. Com 0 saídas → só `Slider(Volume)`.
  - O teste de 144 dpi verifica que a lista cabe dentro do painel.
- [ ] **Step 2:** falham. **Step 3:** implementar (o `audio_menu_*` puro e a fiação). **Step 4:** `cargo test` e build sem avisos.
  - Rodar e abrir com `--painel abrir`. **Não** abra a lista nem mexa no volume (isso mexeria no áudio do usuário).
  - Rode `tools/medir-memoria.ps1`.
- [ ] **Step 5:** Commit `feat: audio dropdown with volume slider`.

---

### Task 5: Medição final, README e checklist

**Files:** Modify `README.md`, `docs/superpowers/plans/2026-11-25-painel-topo-checklist.md`.

- [ ] **Step 1:** `cargo test` e `cargo build --release` sem avisos. Fechar a instância e rodar o exe novo. `tools/medir-memoria.ps1` → anotar os números.
- [ ] **Step 2:** README:
  - Trocar "o consumo do painel ainda não foi medido" pelos números medidos (fechado / aberto / fechado depois de 10 ciclos).
  - Descrever a barra de Transparência na ⚙ e a linha `transparency=N` da config.
  - Descrever a lista suspensa de áudio com a barra de volume.
- [ ] **Step 3:** Checklist:
  - Atualizar os itens de áudio (lista suspensa, barra, ✓ e Esc fechando primeiro a lista), de vidro (barra de transparência) e o tempo de 0,5 s.
  - Acrescentar "arrastar a barra e sair do painel com o botão apertado → não fecha".
  - Marcar como feita a medição de memória, com os números.
- [ ] **Step 4:** Deixar `target\release\battery-bar.exe` rodando. Commit `docs: measured panel memory and updated checklist`.

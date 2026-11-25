# Painel do topo — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Adicionar ao `battery-bar.exe` um painel com vidro fosco do Windows 11. Ele desce da borda de cima ao segurar o mouse 1,5 s e tem busca de apps, Wi‑Fi/Bluetooth/áudio, player, bateria, recursos, calendário e speed test.

**Architecture:** Mesmo processo e mesma thread de UI do battery-bar. Uma janela popup nova (`PanelWindow`) liga o backdrop do DWM e desenha com Direct2D por cima de fundo transparente. Ela é criada ao abrir e destruída ao fechar, para liberar memória. A lógica que dá pra testar fica em módulos puros: gatilho da borda, regra de fechar, layout/hit-test, busca, calendário, agregação de processos, config e cálculo de velocidade. Os módulos de sistema (rádios, áudio, mídia, sysinfo, apps, speed test) só são chamados com o painel aberto. Trabalho bloqueante roda em threads e acorda a UI com `PostMessageW`.

**Tech Stack:** Rust 2024, crate `windows` 0.62 (Win32 + WinRT), Direct2D/DirectWrite, DWM, Core Audio, PDH, WinHTTP, WLAN API.

**Spec:** `docs/superpowers/specs/2026-11-25-painel-topo-design.md`. Leia antes de começar. A seção "Comportamento" é a fonte dos valores.

## Global Constraints

- Repositório `D:\dev\battery-bar`, branch `feat/painel-topo`. Um só `battery-bar.exe`, uma instância (mutex `Local\battery-bar-single` já existe).
- Toolchain: em todo shell, antes de `cargo`: `export RUSTUP_HOME=D:/rust/rustup CARGO_HOME=D:/rust/cargo PATH=/d/rust/cargo/bin:$PATH`.
- **Sem webview nem framework de UI.** Nada de tokio nem cliente HTTP em crate: HTTP é WinHTTP. Dependência nova só se for pequena e justificada no commit.
- Memória (coluna **"Memória"** do Gerenciador de Tarefas): painel fechado ≤ ~8 MB, **inclusive depois de abrir/fechar**. Painel aberto ≤ ~20 MB. CPU ~0% com o painel fechado.
- Painel fechado = nada é medido nem consultado (recursos, mídia, rádios, áudio, apps).
- A faixa da barra de tarefas e as leituras de bateria não mudam de comportamento. `cargo test` dos 72 testes atuais continua passando.
- IDs de mensagem: os existentes são `WM_APP+1..+5`. Os novos ficam em `WM_APP+10` em diante (definidos na Task 2).
- Textos da UI em português, exatamente: `Pesquisar apps…`, `Recursos`, `Calendário`, `Speed test`, `Testar`, `Nada tocando`, `Nada encontrado`, `Indisponível`, `Sem conexão`, `Usar a cor do Windows`, menu da bandeja `Abrir painel`.
- Ícones: *Segoe Fluent Icons* (a faixa já usa). Texto: *Segoe UI Variable Text*.
- Commits em inglês no estilo `feat: ...`, terminando com `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Arrastar janela até o topo** (Snap Layouts do Windows 11 aparece ali): com botão do mouse apertado, o painel **não** pode abrir. → teste `held_button_never_fires` (Task 1).
2. **Escala 125%/150%:** o layout escala com o DPI e nada se sobrepõe. → teste `layout_scales_with_dpi` (Task 3).
3. **PC sem Wi‑Fi, sem Bluetooth ou sem saída de áudio:** o botão aparece `Indisponível`, desabilitado, sem travar. → testes `missing_radio_is_unavailable` (Task 6) e `no_endpoint_is_unavailable` (Task 7).
4. **Busca com acento/maiúscula e sem resultado:** `camera` acha `Câmera`, e uma busca sem resultado mostra `Nada encontrado`. → testes `accent_insensitive` e `no_match_is_empty` (Task 5).
5. **Speed test sem internet ou fechado no meio:** mostra `Sem conexão` e a thread termina. → testes `offline_reports_error` e `cancel_stops_quickly` (Task 11).

---

## Estrutura de arquivos

| Arquivo | Responsabilidade |
| --- | --- |
| `src/panel/mod.rs` | `PanelWindow`: criar/destruir a janela, DWM, foco, entrada (mouse/teclado), desenhar |
| `src/panel/trigger.rs` | puro: `EdgeTrigger` (1,5 s na borda) e `CloseRule` (0,5 s fora + pausas) |
| `src/panel/view.rs` | puro: `PanelView` (tudo que o painel mostra), `Tab`, `Hit` |
| `src/panel/layout.rs` | puro: retângulos de cada área a partir do DPI, `hit_test` |
| `src/panel/draw.rs` | desenha um `PanelView` num `ID2D1RenderTarget` |
| `src/config.rs` | cores: `Config`, parse/serialize `chave=valor`, load/save em `%APPDATA%\battery-bar\config`, cor do Windows |
| `src/search.rs` | puro: normalizar e filtrar apps |
| `src/apps.rs` | listar `shell:AppsFolder`, ícones, abrir app |
| `src/radios.rs` | Wi‑Fi/Bluetooth: estado, ligar/desligar, SSID, aparelhos BT conectados |
| `src/audio.rs` | volume, mudo, saídas, trocar saída (`IPolicyConfig`) |
| `src/media.rs` | música tocando (GSMTC), capa, ⏮ ⏯ ⏭ |
| `src/sysinfo.rs` | CPU/GPU/RAM e processos. Partes puras: `cpu_percent`, `aggregate`, `top` |
| `src/calendar.rs` | puro: grade do mês |
| `src/speedtest.rs` | thread WinHTTP com cancelamento. Puro: `mbps` |
| `src/taskbar.rs` | (mod) expor `pub fn fullscreen_now() -> bool` |
| `src/tray.rs` | (mod) item `CMD_PANEL` "Abrir painel" |
| `src/app.rs` | (mod) timer da borda, abrir/fechar painel, rotear mensagens |

---

### Task 1: Gatilho da borda e regra de fechar (puro)

**Files:**
- Create: `src/panel/mod.rs` (só `pub mod trigger;` por enquanto), `src/panel/trigger.rs`
- Modify: `src/main.rs` (adicionar `mod panel;`)

**Interfaces:**
- Produces:
  - `pub const HOLD_TO_OPEN: Duration = 1500 ms; pub const LEAVE_GRACE: Duration = 500 ms;`
  - `pub struct EdgeTrigger` com `fn new() -> Self` e `fn update(&mut self, at_top_edge: bool, button_down: bool, now: Instant) -> bool`. Devolve `true` uma única vez quando a borda foi segurada por `HOLD_TO_OPEN`, e depois só de novo após o cursor sair da borda. `button_down` ou sair da borda zera o contador.
  - `pub struct Pauses { pub search_text: bool, pub speedtest_running: bool, pub popup_open: bool }`
  - `pub struct CloseRule` com `fn new() -> Self` e `fn update(&mut self, inside: bool, pauses: Pauses, now: Instant) -> bool` (`true` = fechar). Só arma depois do primeiro `inside == true`. Durante pausa não fecha e zera o contador.

- [ ] **Step 1: Escrever os testes** em `trigger.rs` (`#[cfg(test)]`), com `t0 = Instant::now()` e somas de `Duration`:
  - `fires_after_hold`: `update(true,false,t0)` → false; em `t0+1499ms` → false; em `t0+1500ms` → true.
  - `fires_once_per_visit`: depois de disparar, em `t0+3s` com `at_top_edge=true` → false; sai (`false`) e volta segurando 1,5 s → true de novo.
  - `leaving_resets`: borda em t0, fora em t0+1s, borda em t0+1,1s; em t0+2s → false; em t0+2,6s → true.
  - `held_button_never_fires`: borda com `button_down=true` de t0 até t0+5s → sempre false. Soltar o botão em t0+5s → dispara em t0+6,5s.
  - `close_not_armed_before_entering`: `CloseRule` com `inside=false` por 5 s → sempre false.
  - `close_after_grace`: entra em t0, sai em t0+1s; em t0+1,4s → false; em t0+1,5s → true.
  - `reentering_cancels_close`: sai em t0+1s, volta em t0+1,3s, sai de novo em t0+2s → false em t0+2,4s.
  - `pauses_block_close`: armado e fora há 3 s, mas com `search_text=true` → false. Tirar a pausa reinicia a folga: só fecha 0,5 s depois.
- [ ] **Step 2:** `cargo test panel::trigger` → falha (tipos não existem).
- [ ] **Step 3:** Implementar `EdgeTrigger` e `CloseRule` (estado `Option<Instant>` + `bool` de "já disparou/armado").
- [ ] **Step 4:** `cargo test panel::trigger` → 8 passam. `cargo test` → todos passam.
- [ ] **Step 5:** Commit `feat: edge trigger and close rule for the top panel`.

---

### Task 2: Janela do painel com vidro, abrir/fechar e foco

**Files:**
- Modify: `src/panel/mod.rs`, `src/app.rs`, `src/taskbar.rs`, `src/tray.rs`, `Cargo.toml`

**Interfaces:**
- Consumes: `EdgeTrigger`, `CloseRule`, `Pauses` (Task 1).
- Produces:
  - Em `taskbar.rs`: `pub fn fullscreen_now() -> bool` (combina `foreground_is_fullscreen` com o monitor principal + `shell_state`, usando o `fullscreen_active` existente).
  - Em `tray.rs`: `pub const CMD_PANEL: u32 = 4;`. O item "Abrir painel" é o primeiro do menu.
  - Em `panel/mod.rs`: `pub const WM_PANEL_ASYNC: u32 = WM_APP + 10;` (threads do painel avisando que há dado novo; `wParam` = origem, ver tasks seguintes) e `pub const WM_PANEL_CLOSED: u32 = WM_APP + 11;` (o painel se fechou sozinho: Esc, clique fora, perda de ativação).
  - `pub struct PanelWindow` com `fn open(owner: HWND) -> windows::core::Result<Self>`, `fn hwnd(&self) -> HWND`, `fn redraw(&mut self)` e `Drop`, que destrói a janela, solta o render target e devolve o foco a quem tinha antes.
  - Em `app.rs`: `App` ganha `panel: Option<PanelWindow>`, `edge: EdgeTrigger`, `close: CloseRule`. Também ganha `fn open_panel(&mut self, hwnd: HWND)` e `fn close_panel(&mut self)`. O último faz `self.panel = None` e depois `SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX)`.
- Cargo features novas: `Win32_Graphics_Dwm`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_System_ProcessStatus`.

- [ ] **Step 1: Teste puro do posicionamento.** `pub fn panel_rect(monitor: RECT, width_px: i32, height_px: i32) -> RECT` em `panel/mod.rs`: centralizado horizontalmente, `top == monitor.top`. Testes `panel_centered_at_top` (monitor 0..1920, largura 760 → left 580, top 0) e `panel_clamped_on_small_monitor` (largura maior que o monitor → `left == monitor.left`, `right == monitor.right`).
- [ ] **Step 2:** `cargo test panel::tests` → falha.
- [ ] **Step 3: Implementar a janela.** Classe própria e estilo `WS_POPUP`, com `WS_EX_TOOLWINDOW | WS_EX_TOPMOST` (sem `WS_EX_LAYERED`, que bloqueia o backdrop). Ao criar:
  - `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW)`
  - `DWMWA_USE_IMMERSIVE_DARK_MODE = 1`
  - `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`
  - `DwmExtendFrameIntoClientArea` com margens `-1`
  - Direct2D desenhando com `Clear` transparente. O tipo de render target fica a critério do construtor (spec, "Deixado pro construtor"); o critério é o vidro aparecer atrás do que é desenhado.

  Tamanho inicial 760×420 DIP vezes `dpi/96`, monitor principal (`MonitorFromPoint(0,0, MONITOR_DEFAULTTOPRIMARY)`). Por enquanto desenha só um retângulo arredondado de teste com o texto `Pesquisar apps…`.
- [ ] **Step 4: Foco.** Em `open`: guarda `GetForegroundWindow()` e faz `AttachThreadInput(thread_da_janela_ativa, thread_atual, true)` → `SetForegroundWindow(panel)` → `SetFocus(panel)` → `AttachThreadInput(.., false)`. **Nunca** usar `SendInput`. No `Drop`, `SetForegroundWindow(anterior)` se ela ainda existir. Clique no painel com `WM_MOUSEACTIVATE` → `MA_ACTIVATE`, para que um clique pegue o foco se o Windows tiver negado.
- [ ] **Step 5: Abrir e fechar no `app.rs`.**
  - Novo `TIMER_EDGE = 3` de **100 ms**, sempre ligado. A cada tick: `GetCursorPos` + rect do monitor principal. `at_top_edge = pt.y <= monitor.top`. `button_down = GetAsyncKeyState(VK_LBUTTON|VK_RBUTTON) < 0`.
  - Se `edge.update(..)` devolver true, o painel estiver fechado e `!taskbar::fullscreen_now()`, chama `open_panel`.
  - Com o painel aberto, o mesmo tick chama `close.update(cursor dentro do rect do painel, pauses, now)`. As pausas vêm do estado do painel; até a Task 5, todas `false`. Se devolver true, chama `close_panel`.
  - No painel, `Esc` (`WM_KEYDOWN VK_ESCAPE`) e `WM_ACTIVATE` com `WA_INACTIVE` (clique fora) postam `WM_PANEL_CLOSED` ao dono, que chama `close_panel`.
  - `WM_DISPLAYCHANGE` com o painel aberto → `close_panel`.
  - Menu da bandeja `CMD_PANEL` → `open_panel`, sem checar a borda. O `CloseRule` começa desarmado (Task 1).
- [ ] **Step 6:** `cargo test` → tudo passa. `cargo build --release`.
- [ ] **Step 7: Verificação manual** (anotar o resultado no commit):
  1. Mouse 1,5 s no topo → o painel desce com vidro fosco, e o papel de parede aparece desfocado atrás.
  2. Mouse no topo por 1 s, depois clicar numa aba do Chrome maximizado → não abre.
  3. Arrastar uma janela até o topo (aparece o Snap) → não abre.
  4. Esc, clique fora e mouse fora por 0,5 s → fecha.
  5. Jogo/vídeo em tela cheia (F11 no YouTube) → não abre.
  6. Ao abrir, digitar uma tecla → o painel recebe `WM_CHAR` (`OutputDebugString` temporário).
  7. Gerenciador de Tarefas, coluna Memória: anotar o valor fechado, aberto e fechado de novo após 5 ciclos. Fechado deve ficar ≤ ~8 MB.
- [ ] **Step 8:** Commit `feat: top panel window with system glass, edge open and focus`.

---

### Task 3: Modelo da tela, layout e desenho

**Files:**
- Create: `src/panel/view.rs`, `src/panel/layout.rs`, `src/panel/draw.rs`
- Modify: `src/panel/mod.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `PanelWindow` (Task 2). Pega `model::{DeviceKind, ItemView}` e `Tracker::view` do código atual.
- Produces (`view.rs`). As tasks seguintes preenchem os campos; até lá, valores padrão:
  ```rust
  pub enum Tab { Resources, Calendar, Speed }
  pub enum Toggle { Unavailable, Off, On }
  pub struct QuickState { pub wifi: Toggle, pub wifi_name: Option<String>,
      pub bt: Toggle, pub bt_connected: u32,
      pub audio: Toggle /* On = com som, Off = mudo */, pub volume: u8, pub output_name: Option<String> }
  pub enum Hit { Search, Gear, Wifi, WifiMore, Bluetooth, BtMore, Audio, AudioMore,
      AudioDevice(usize), MediaPrev, MediaPlay, MediaNext, Tab(Tab), SortCpu, SortRam,
      CalPrev, CalNext, SpeedStart, Accent(usize), AccentWindows, Tint(usize), Result(usize) }
  pub struct PanelView { pub dpi: u32, pub colors: crate::config::Colors, pub quick: QuickState,
      pub battery: Vec<(DeviceKind, ItemView)>, pub media: Option<crate::media::NowPlaying>,
      pub tab: Tab, pub query: String, pub results: Vec<crate::apps::AppEntry>, pub selected: usize,
      pub stats: Option<crate::sysinfo::Stats>, pub sort_ram: bool,
      pub month: (i32, u32), pub speed: crate::speedtest::SpeedState,
      pub gear_open: bool, pub audio_list: Option<Vec<String>>, pub hover: Option<Hit> }
  ```
  Para compilar antes das tasks deles, crie stubs mínimos com `Default` em `config.rs` (`Colors`), `media.rs` (`NowPlaying`), `apps.rs` (`AppEntry`), `sysinfo.rs` (`Stats`) e `speedtest.rs` (`SpeedState`). Cada task depois completa o seu.
- Produces (`layout.rs`): `pub struct Layout { pub size: (f32, f32), pub regions: Vec<(D2D_RECT_F, Hit)>, pub search: D2D_RECT_F, pub quick_row: D2D_RECT_F, pub battery: D2D_RECT_F, pub media: D2D_RECT_F, pub tabs: D2D_RECT_F, pub body: D2D_RECT_F }`, mais `pub fn layout(view: &PanelView) -> Layout` (em pixels, já com DPI) e `pub fn hit_test(l: &Layout, x: f32, y: f32) -> Option<Hit>`.
  - Medidas base em DIP: largura 760; padding 16; busca h 36; linha rápida h 52 (Wi‑Fi, BT e Áudio com 120 cada, cada um com sua "setinha" de 28 à direita, e bateria ocupando o resto); player h 44; abas h 28; corpo h 220. Com `query` não vazia, o corpo vira a lista (8 linhas de 30). Com `gear_open`, aparece uma faixa extra de 40 abaixo da busca.
- Produces (`draw.rs`): `pub fn draw(rt: &ID2D1RenderTarget, dw: &IDWriteFactory, view: &PanelView, layout: &Layout) -> windows::core::Result<()>`.

- [ ] **Step 1: Testes do layout:**
  - `layout_has_all_fixed_regions`: `PanelView::default()` com dpi 96 → `hit_test` no centro da busca = `Search`, no centro do Wi‑Fi = `Wifi` e na setinha = `WifiMore`.
  - `layout_scales_with_dpi`: dpi 144 → `size.0 == 760*1.5`, e nenhum par de `regions` se sobrepõe.
  - `query_switches_body_to_results`: com `query="a"` e 3 resultados → existem `Result(0..3)`, e nenhum `Tab(_)` nem `SortCpu` no corpo.
  - `gear_adds_color_row`: `gear_open=true` → aparecem `Accent(0..8)`, `AccentWindows` e `Tint(0..3)`, e `size.1` cresce 40 DIP.
- [ ] **Step 2:** `cargo test panel::layout` → falha.
- [ ] **Step 3:** Implementar `layout`, `hit_test` e `draw`.
  - Cards com retângulo arredondado de raio 8 DIP e fundo `#FFFFFF` a 7% (vidro escuro).
  - Toggle `On` com a cor de destaque; `Off` como card; `Unavailable` com o texto `Indisponível` a 45% de opacidade.
  - Bateria com os mesmos glifos e regras da faixa: reutilize `device_glyph` e as cores de `render.rs`, tornando-os `pub(crate)`.
  - Texto com antialias ClearType desligado, igual à faixa.
  - Altura da janela = `layout.size.1`. Redimensionar quando mudar.
- [ ] **Step 4:** Ligar no `PanelWindow`:
  - `WM_PAINT`/`redraw` montam o `PanelView` a partir do `App`. A bateria vem de `tracker.view(k, now)` para `ALL`.
  - `WM_MOUSEMOVE` atualiza `hover` e redesenha só se mudar.
  - `WM_LBUTTONUP` chama `hit_test` e manda o `Hit` para `App::on_hit(hit)`. Por enquanto só `Tab(_)` e `Gear` mudam estado.
  - `WM_READING` com o painel aberto redesenha o painel também.
- [ ] **Step 5:** `cargo test` → passa. Rodar e conferir que o painel mostra a bateria igual à faixa e que as abas trocam.
- [ ] **Step 6:** Commit `feat: panel view model, layout and drawing`.

---

### Task 4: Cores e configuração

**Files:**
- Create/complete: `src/config.rs`
- Modify: `src/panel/draw.rs`, `src/app.rs`

**Interfaces:**
- Produces:
  - `pub const ACCENTS: [u32; 8] = [0x9BE3B0, 0x60CDFF, 0xB4A0FF, 0xFF8FB1, 0xFFB86B, 0xF5E07A, 0x7FE0D4, 0xE6E6E6];`
  - `pub enum Accent { Preset(usize), Windows, Custom(u32) }`
  - `pub enum GlassTint { Dark, Light, Accent }` (`Tint(0..3)` na ordem)
  - `pub struct Config { pub accent: Accent, pub tint: GlassTint }`. `Default` = `Preset(0)` + `Dark`.
  - `pub fn parse(text: &str) -> Config`, `pub fn serialize(c: &Config) -> String`
  - `pub fn load() -> Config` e `pub fn save(c: &Config)`, em `%APPDATA%\battery-bar\config`, criando a pasta.
  - `pub fn windows_accent() -> u32`: lê `HKCU\Software\Microsoft\Windows\DWM\AccentColor` (DWORD em ABGR) e devolve RGB. Se não achar, usa `ACCENTS[0]`.
  - `pub struct Colors { pub accent: u32, pub tint: (u32, f32) }` e `pub fn resolve(c: &Config) -> Colors`. Tints: Dark = `(0x000000, 0.25)`, Light = `(0xFFFFFF, 0.18)`, Accent = `(accent, 0.15)`.
- Formato do arquivo (`chave=valor`, uma por linha, `#` comenta):
  ```
  accent=windows        # ou preset:3, ou #FF8800
  tint=dark             # dark | light | accent
  ```

- [ ] **Step 1: Testes:**
  - `roundtrip`: `parse(serialize(c)) == c` para `Preset(3)/Light`, `Windows/Dark` e `Custom(0xFF8800)/Accent`.
  - `garbage_falls_back_to_default`: `parse("accent=banana\ntint=")` == `Config::default()`.
  - `comments_and_spaces`: `parse("  accent = #ff8800  # laranja\n")` → `Custom(0xFF8800)`.
  - `abgr_to_rgb`: `pub fn abgr_to_rgb(0xFF332211) == 0x112233`.
- [ ] **Step 2:** `cargo test config` → falha.
- [ ] **Step 3:** Implementar. O `App` carrega o `Config` ao abrir o painel.
  - `Hit::Accent(i)` / `AccentWindows` / `Tint(i)`: muda, chama `save` e redesenha na hora.
  - `draw` pinta o tint como retângulo cheio por cima do vidro e usa `colors.accent` em toggles `On`, barras, aba ativa e dia de hoje.
  - Com `Accent::Windows`, `WM_SETTINGCHANGE "ImmersiveColorSet"` com o painel aberto chama `resolve` de novo.
- [ ] **Step 4:** `cargo test` → passa. Manual: trocar a cor na ⚙, fechar e abrir de novo → a cor continua. O arquivo existe e dá pra editar à mão.
- [ ] **Step 5:** Commit `feat: accent and glass tint settings for the panel`.

---

### Task 5: Busca de apps

**Files:**
- Create: `src/search.rs`, `src/apps.rs` (completar o stub)
- Modify: `src/panel/mod.rs`, `src/app.rs`, `Cargo.toml`

**Interfaces:**
- Produces:
  - `search.rs`: `pub fn normalize(s: &str) -> String` (minúsculas e sem acentos: mapeie á/à/â/ã/ä, é/ê/è/ë, í/î/ì/ï, ó/ô/õ/ò/ö, ú/ù/û/ü, ç, ñ) e `pub fn filter<'a>(apps: &'a [AppEntry], query: &str, max: usize) -> Vec<&'a AppEntry>`. A ordem é: quem começa com o texto, depois quem tem uma palavra que começa com ele, depois quem só contém. Dentro de cada grupo, ordem alfabética por `normalize(name)`.
  - `apps.rs`: `#[derive(Clone)] pub struct AppEntry { pub name: String, pub id: String /* AppUserModelID ou parsing name */, pub icon: Option<HBITMAP-ish> }`. O tipo do ícone fica a critério do construtor, mas precisa ser liberado no `Drop` do painel. Também `pub fn enumerate() -> Vec<AppEntry>` (itens de `FOLDERID_AppsFolder` via `SHGetKnownFolderItem` + `IEnumShellItems`, nome de exibição `SIGDN_NORMALDISPLAY` e id `SIGDN_PARENTRELATIVEPARSING`) e `pub fn launch(id: &str)` (`ShellExecuteW("open", "shell:AppsFolder\\" + id)`). Ícones de 32 px via `IShellItemImageFactory::GetImage`, carregados **só para os resultados visíveis**.
  - `Pauses.search_text = !view.query.is_empty()`.
- Cargo features: `Win32_UI_Shell_Common`, `Win32_System_Com_StructuredStorage`, `Win32_UI_Shell_PropertiesSystem`.

- [ ] **Step 1: Testes** em `search.rs`, com uma lista fixa: `["Câmera", "Calculadora", "Visual Studio Code", "Discord", "Steam", "Spotify", "OBS Studio"]`:
  - `accent_insensitive`: `filter(.., "camera", 8)` → `["Câmera"]`.
  - `prefix_before_word_before_contains`: `"st"` → `["Steam", "OBS Studio", "Visual Studio Code"]`.
  - `case_insensitive`: `"DIS"` → `["Discord"]`.
  - `no_match_is_empty`: `"zzz"` → vazio.
  - `max_limits`: `"s"` com max 2 → só 2 itens.
- [ ] **Step 2:** `cargo test search` → falha.
- [ ] **Step 3:** Implementar `search.rs`.
- [ ] **Step 4:** `cargo test search` → 5 passam.
- [ ] **Step 5:** Implementar `apps.rs` e ligar no painel.
  - `enumerate` roda numa thread ao abrir o painel. O resultado chega por canal + `WM_PANEL_ASYNC` (`wParam = 1`) e fica guardado no `App` enquanto o painel está aberto; é descartado ao fechar.
  - `WM_CHAR` imprimível acrescenta à `query`, e Backspace apaga.
  - ↑/↓ mudam `selected`. Enter ou `Hit::Result(i)` chamam `launch` e fecham o painel.
  - Com a query vazia, volta às abas. Com a query preenchida e sem resultados, mostra `Nada encontrado` no corpo.
- [ ] **Step 6:** Manual: digitar `calc` + Enter abre a Calculadora (app da Store), e `code` acha o VS Code (Win32). Abrir → digitar → apagar tudo → as abas voltam. Com texto na busca, mouse fora → não fecha.
- [ ] **Step 7:** Commit `feat: app search in the panel`.

---

### Task 6: Wi‑Fi e Bluetooth

**Files:**
- Create: `src/radios.rs`
- Modify: `src/app.rs`, `Cargo.toml`

**Interfaces:**
- Produces:
  - `pub fn to_toggle(present: bool, on: bool) -> Toggle` (puro).
  - `pub struct RadioSnapshot { pub wifi: Toggle, pub wifi_name: Option<String>, pub bt: Toggle, pub bt_connected: u32 }`
  - `pub fn snapshot() -> RadioSnapshot` (bloqueante, roda em thread). Usa `Windows.Devices.Radios.Radio::GetRadiosAsync()`, com `.get()` na thread, para estado e presença. O SSID vem de `WlanOpenHandle` + `WlanQueryInterface(wlan_intf_opcode_current_connection)`. Os aparelhos BT conectados vêm de `BluetoothFindFirstDevice` com `fReturnConnected = TRUE` e são contados.
  - `pub fn set(kind: RadioKind, on: bool)` (thread). `pub enum RadioKind { Wifi, Bluetooth }`.
  - Resultado entregue com `WM_PANEL_ASYNC` (`wParam = 2`).
- Cargo features: `Devices_Radios`, `Foundation`, `Foundation_Collections`, `Win32_NetworkManagement_WiFi`, `Win32_Devices_Bluetooth`.

- [ ] **Step 1: Testes:** `missing_radio_is_unavailable` (`to_toggle(false, true) == Unavailable`), `present_off_is_off`, `present_on_is_on`.
- [ ] **Step 2:** `cargo test radios` → falha. **Step 3:** implementar `to_toggle`. **Step 4:** passa.
- [ ] **Step 5:** Implementar `snapshot`/`set` e ligar.
  - Ao abrir o painel, roda `snapshot`.
  - `Hit::Wifi`/`Bluetooth`: chama `set(!on)` e depois `snapshot` de novo.
  - `Unavailable` ignora o clique.
  - `WifiMore` abre `ms-settings:network-wifi`, `BtMore` abre `ms-settings:bluetooth`, e clique direito no botão faz o mesmo. Os dois fecham o painel.
  - Falhas de API viram `Unavailable` e nunca causam pânico.
- [ ] **Step 6:** Manual: desligar/ligar o Wi‑Fi pelo painel e conferir no Win+A; o nome da rede aparece. Bluetooth com o DualSense ligado → `1 conectado`.
- [ ] **Step 7:** Commit `feat: wifi and bluetooth quick toggles`.

---

### Task 7: Áudio

**Files:**
- Create: `src/audio.rs`
- Modify: `src/app.rs`, `Cargo.toml`

**Interfaces:**
- Produces:
  - `pub struct AudioSnapshot { pub state: Toggle, pub volume: u8, pub output: Option<String>, pub outputs: Vec<(String /*id*/, String /*nome*/)> }`
  - `pub fn snapshot() -> AudioSnapshot`: `IMMDeviceEnumerator`, `GetDefaultAudioEndpoint(eRender, eMultimedia)`, `IAudioEndpointVolume` e nome via `PKEY_Device_FriendlyName`. Sem endpoint → `state = Unavailable`.
  - `pub fn step_volume(delta: i32)` (passo de 2% por notch da roda), `pub fn toggle_mute()` e `pub fn set_default(id: &str) -> bool`. O último usa `IPolicyConfig`, declarada à mão com `#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]`, e chama `SetDefaultEndpoint` para `eConsole`, `eMultimedia` e `eCommunications`.
  - `pub fn clamp_volume(current: u8, delta: i32) -> u8` (puro).
  - `Pauses.popup_open = view.gear_open || view.audio_list.is_some()`.
- Cargo features: `Win32_Media_Audio`, `Win32_Media_Audio_Endpoints`, `Win32_Devices_FunctionDiscovery`, `Win32_System_Com_StructuredStorage`.

- [ ] **Step 1: Testes:** `clamp_volume(98, 4) == 100`, `clamp_volume(1, -4) == 0` e `no_endpoint_is_unavailable`. Para o último, `pub fn snapshot_from(endpoint: Option<(bool /*mudo*/, u8, String)>, outputs: Vec<..>) -> AudioSnapshot`, que com `None` dá `Unavailable`. O `snapshot()` real chama `snapshot_from`.
- [ ] **Step 2:** falha. **Step 3:** implementar o puro. **Step 4:** passa.
- [ ] **Step 5:** Implementar e ligar.
  - `WM_MOUSEWHEEL` sobre `Hit::Audio` chama `step_volume(±2 por notch)` + `snapshot`.
  - Clique em `Audio` chama `toggle_mute`.
  - `AudioMore` abre a lista de saídas no corpo (`audio_list`). `AudioDevice(i)` chama `set_default`; se der `false`, abre `ms-settings:sound` e fecha o painel.
  - Clique direito em `Audio` abre `ms-settings:sound`.
- [ ] **Step 6:** Manual: rolar o mouse muda o volume (conferir no ícone do Windows), mudo funciona, e trocar entre o fone USB e outra saída funciona.
- [ ] **Step 7:** Commit `feat: audio volume, mute and output switching`.

---

### Task 8: Player

**Files:**
- Create/complete: `src/media.rs`
- Modify: `src/app.rs`, `src/panel/draw.rs`, `Cargo.toml`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Default)] pub struct NowPlaying { pub title: String, pub artist: String, pub playing: bool, pub cover: Option<Vec<u8> /*BGRA 48x48*/> }`
  - `pub enum MediaCmd { Prev, PlayPause, Next }`
  - `pub fn current() -> Option<NowPlaying>`: `GlobalSystemMediaTransportControlsSessionManager::RequestAsync().get()`, depois `GetCurrentSession`, `TryGetMediaPropertiesAsync` e `GetPlaybackInfo`. A capa vem de `Thumbnail` → stream → WIC, reduzida para 48×48 BGRA. Se falhar, `cover = None`.
  - `pub fn send(cmd: MediaCmd)`.
  - Entrega por `WM_PANEL_ASYNC` (`wParam = 3`). Com o painel aberto, consulta ao abrir, depois de cada comando e a cada 2 s. Fechado, não consulta.
- Cargo features: `Media_Control`, `Storage_Streams`, `Win32_Graphics_Imaging`.

- [ ] **Step 1:** `current` e `send` em thread, ligados a `MediaPrev/Play/Next`. `draw` mostra a capa (bitmap D2D criado do BGRA) + título · artista. Glifo ⏯ muda conforme `playing`. `None` → `Nada tocando` com 55% de opacidade.
- [ ] **Step 2:** `cargo build`. Manual: Spotify ou YouTube no navegador tocando → aparece título/artista/capa; ⏯ pausa; ⏭ pula. Nada tocando → `Nada tocando`.
- [ ] **Step 3:** Commit `feat: now playing and media controls in the panel`.

---

### Task 9: Aba Recursos

**Files:**
- Create/complete: `src/sysinfo.rs`
- Modify: `src/app.rs`, `src/panel/draw.rs`, `Cargo.toml`

**Interfaces:**
- Produces:
  - `pub struct ProcRow { pub name: String, pub cpu: f32 /*%*/, pub ram: u64 /*bytes*/ }`
  - `#[derive(Default)] pub struct Stats { pub cpu: f32, pub gpu: Option<f32>, pub ram_used: u64, pub ram_total: u64, pub top: Vec<ProcRow> }`
  - Puros:
    - `pub fn cpu_percent(prev: (u64 idle, u64 kernel, u64 user), cur: (..)) -> f32` (`busy = (k+u) - idle`; `100 * busy / (k+u)`, com 0 se o delta for 0).
    - `pub fn aggregate(rows: Vec<ProcRow>) -> Vec<ProcRow>`: soma por nome sem diferenciar maiúsculas e **sem** `.exe`, ignorando `Idle`/`System Idle Process`.
    - `pub fn top(rows: Vec<ProcRow>, by_ram: bool, n: usize) -> Vec<ProcRow>`.
  - `pub struct Sampler`, com `fn new() -> Self` e `fn sample(&mut self) -> Stats`.
    - CPU total: `GetSystemTimes`.
    - RAM: `GlobalMemoryStatusEx`.
    - Processos: `CreateToolhelp32Snapshot`, depois `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`, `GetProcessTimes` (delta / tempo × núcleos) e `GetProcessMemoryInfo.PrivateUsage`. Acesso negado → pula.
    - GPU: consulta PDH `\GPU Engine(*)\Utilization Percentage`. Some os valores por motor `engtype_*` e use o **maior** total entre os tipos de motor (3D, Copy, VideoDecode…), limitado a 100. Sem contador → `None`.
  - O `Sampler` vive só enquanto o painel está aberto **e** `tab == Resources`. Timer `TIMER_STATS = 4` de 1000 ms, ligado e desligado junto com ele.
- Cargo features: `Win32_System_Performance`, `Win32_System_Diagnostics_ToolHelp`, `Win32_System_SystemInformation`.

- [ ] **Step 1: Testes:**
  - `cpu_percent((100,200,100),(150,300,200)) == 75.0`, e delta zero → `0.0`.
  - `aggregate_sums_same_name`: `chrome.exe` 10%/100 + `Chrome.exe` 5%/50 → uma linha `chrome` 15%/150.
  - `aggregate_drops_idle`.
  - `top_by_cpu_then_ram`: a ordem muda com `by_ram`, e `n=5` corta.
- [ ] **Step 2:** falha. **Step 3:** implementar o puro. **Step 4:** passa.
- [ ] **Step 5:** Implementar o `Sampler` e ligar.
  - `draw` mostra CPU/GPU/RAM com barra (cor de destaque) e 5 linhas `nome · CPU% · RAM`. GPU `None` aparece como `—`.
  - `SortCpu`/`SortRam` alternam `sort_ram`.
  - A primeira amostra só sai no segundo tick, porque a CPU precisa de delta. Até lá aparece `…`.
- [ ] **Step 6:** Manual: comparar com o Gerenciador de Tarefas (diferença de alguns %). Abrir um jogo → a GPU sobe. Painel fechado ou noutra aba → CPU do battery-bar ~0% (Process Explorer ou Gerenciador).
- [ ] **Step 7:** Commit `feat: resources tab with cpu, gpu, ram and top processes`.

---

### Task 10: Aba Calendário

**Files:**
- Create: `src/calendar.rs`
- Modify: `src/app.rs`, `src/panel/draw.rs`

**Interfaces:**
- Produces:
  - `pub fn days_in_month(y: i32, m: u32) -> u32`
  - `pub fn weekday_of_first(y: i32, m: u32) -> u32` (0 = domingo; algoritmo de Zeller/Sakamoto, sem crate de data)
  - `pub fn month_grid(y: i32, m: u32) -> [[Option<u8>; 7]; 6]`
  - `pub fn shift(month: (i32, u32), delta: i32) -> (i32, u32)`
  - Hoje vem de `GetLocalTime`. `view.month` é posto no mês atual a cada abertura do painel.

- [ ] **Step 1: Testes:**
  - `days_in_month(2024,2)==29`, `(2026,2)==28`, `(1900,2)==28`, `(2000,2)==29`.
  - `weekday_of_first(2026,11)==0` (1/11/2026 é domingo) e `(2026,12)==2`.
  - `month_grid(2026,11)[0][0]==Some(1)` e `[4][1]==Some(30)`; o resto da linha 4 é `None`.
  - `shift((2026,12),1)==(2027,1)`, `shift((2026,1),-1)==(2025,12)`.
- [ ] **Step 2:** falha. **Step 3:** implementar. **Step 4:** passa.
- [ ] **Step 5:** `draw`: título `Novembro 2026` com meses em português e cabeçalho `D S T Q Q S S`. Hoje fica num círculo da cor de destaque. `CalPrev`/`CalNext` chamam `shift`.
- [ ] **Step 6:** Commit `feat: calendar tab`.

---

### Task 11: Aba Speed test

**Files:**
- Create/complete: `src/speedtest.rs`
- Modify: `src/app.rs`, `src/panel/draw.rs`, `Cargo.toml`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Default)] pub enum SpeedState { #[default] Idle, Running { phase: Phase, ping_ms: Option<u32>, down: Option<f32>, up: Option<f32> }, Done { ping_ms: u32, down: f32, up: f32, at: (u16,u16) /*hh,mm*/ }, Error(String) }`, com `pub enum Phase { Ping, Download, Upload }`.
  - `pub fn mbps(bytes: u64, elapsed: Duration) -> f32` (`bytes*8/1e6/secs`; 0 se elapsed é 0).
  - `pub fn start(hwnd_owner: isize, cancel: Arc<AtomicBool>, tx: Sender<SpeedState>)`, que cria a thread. Ela segue estes passos:
    1. Ping: 5× `GET https://speed.cloudflare.com/__down?bytes=0`, e fica a mediana do tempo.
    2. Download: `__down?bytes=25000000` lido em blocos de 64 KB.
    3. Upload: `POST /__up` com 10 MB de zeros.
    
    Manda `Running` a cada ~250 ms e `Done`/`Error` no fim (`WM_PANEL_ASYNC`, `wParam = 4`). Checa `cancel` entre blocos. Falha de conexão → `Error("Sem conexão")`.
  - `pub fn run_with(client: &mut dyn Http, cancel: &AtomicBool, report: &mut dyn FnMut(SpeedState))`, com `pub trait Http { fn get(&mut self, path: &str, sink: &mut dyn FnMut(usize) -> bool) -> Result<(), ()>; fn post(&mut self, path: &str, bytes: usize, tick: &mut dyn FnMut(usize) -> bool) -> Result<(), ()>; }`. Os callbacks devolvem `false` para abortar. `start` usa a implementação WinHTTP; os testes usam uma falsa.
  - O último resultado fica no `App` até o processo terminar. `Pauses.speedtest_running = matches!(speed, Running{..})`. Fechar o painel seta `cancel`.
- Cargo features: `Win32_Networking_WinHttp`.

- [ ] **Step 1: Testes** com um `Http` falso:
  - `mbps(12_500_000, 1s) == 100.0`.
  - `offline_reports_error`: o falso devolve `Err` no primeiro `get` → último report é `Error("Sem conexão")`.
  - `cancel_stops_quickly`: o falso entrega blocos sem fim, e `cancel` é setado depois de 3 blocos → `run_with` retorna, e o último report não é `Done`.
  - `full_run_reports_done`: o falso entrega tudo → o último report é `Done` com `down > 0` e `up > 0`.
- [ ] **Step 2:** falha. **Step 3:** implementar `mbps` e `run_with`. **Step 4:** passa.
- [ ] **Step 5:** Implementar o `Http` com WinHTTP: `WinHttpOpen` com agent `battery-bar`, `WinHttpConnect("speed.cloudflare.com", 443)` e `WINHTTP_FLAG_SECURE`.
  - Ligar `SpeedStart` (o botão `Testar`, desabilitado enquanto roda).
  - `draw` mostra a fase atual e os números, ou `Error` em vermelho.
- [ ] **Step 6:** Manual: o resultado fica próximo do speedtest do navegador. Fechar o painel no meio → no Gerenciador, a conexão cai e a CPU volta a ~0. Com o Wi‑Fi desligado → `Sem conexão`.
- [ ] **Step 7:** Commit `feat: speed test tab`.

---

### Task 12: Acabamento e verificação final

**Files:**
- Modify: `README.md`, e o que a verificação apontar

- [ ] **Step 1:** `cargo test` → tudo passa. `cargo build --release` sem warnings novos.
- [ ] **Step 2: Checklist manual** com o `.exe` de release:
  - Os 7 itens da Task 2.
  - Escala do Windows em 150% → painel nítido, sem sobreposição.
  - Explorer reiniciado com o painel fechado → abrir pela borda continua funcionando.
  - A faixa da barra de tarefas continua igual.
- [ ] **Step 3: Memória** (coluna "Memória" do Gerenciador): anotar o valor fechado no início, aberto em cada aba e fechado depois de 10 ciclos abrir/fechar passando por todas as abas. Fechado ≤ ~8 MB e aberto ≤ ~20 MB. Se fechado passar do limite, investigar o que não está sendo solto antes de seguir.
- [ ] **Step 4:** README: seção "Painel do topo" com como abrir (1,5 s no topo ou "Abrir painel" na bandeja), as abas, a ⚙ e o arquivo `%APPDATA%\battery-bar\config` (formato). Atualizar a linha de consumo com os números medidos.
- [ ] **Step 5:** Commit `docs: readme for the top panel`.

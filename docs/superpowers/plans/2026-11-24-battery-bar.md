# battery-bar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Um `.exe` Rust leve que desenha ícone + % de bateria do mouse Zaopin, do DualSense e de um celular Android numa faixa fixa na barra de tarefas do Windows 11, à esquerda da bandeja.

**Architecture:** Uma thread de UI Win32 (janela layered sobre a barra + ícone na bandeja) e uma thread por leitor (DualSense via `hidapi`, celular via `adb.exe`, presença do receptor Zaopin). Leitores mandam `(DeviceKind, ReadOutcome)` por `mpsc` e acordam a UI com `PostMessageW`. A UI guarda o estado num `Tracker` puro (testável), desenha com Direct2D/DirectWrite num bitmap BGRA premultiplicado e apresenta com `UpdateLayeredWindow`.

**Tech Stack:** Rust stable (`x86_64-pc-windows-gnu`), crates `windows` (Win32 + Direct2D + DirectWrite + UIA), `hidapi` (backend `windows-native`, sem C).

**Spec:** `docs/superpowers/specs/2026-11-24-battery-bar-design.md` — leia antes de começar.

## Global Constraints

- Projeto em `D:\dev\battery-bar`, binário `battery-bar.exe`, `#![windows_subsystem = "windows"]`.
- Toolchain: em todo shell, antes de `cargo`: `export RUSTUP_HOME=D:/rust/rustup CARGO_HOME=D:/rust/cargo PATH=/d/rust/cargo/bin:$PATH` (o PATH do usuário já tem, mas o shell da sessão pode não ter).
- `adb.exe` está em `D:\android\platform-tools` (no PATH do usuário).
- Sem framework de interface (nada de egui/GPUI/winit/tray-icon). Só `windows` e `hidapi` como dependências de runtime; qualquer outra precisa de justificativa no commit.
- Meta: < 10 MB de RAM do nosso processo, CPU ~0% em repouso.
- Intervalos: mouse 60 s, controle 60 s, celular 180 s. "Baixo" = `< 20%`. Valor mantido até 10 min sem leitura boa.
- Ordem na faixa: mouse, controle, celular.
- **DualSense: nunca enviar nada ao controle** (nem feature report `0x05`, nem output report). Só ler input reports.
- **Zaopin: nunca enviar nada ao receptor** neste plano. Só enumerar.
- Fontes: ícones *Segoe Fluent Icons* — mouse `U+E962`, gamepad `U+E7FC`, celular `U+E8EA`, raio `U+E945`, ícone da bandeja (bateria) `U+E83F`. Texto *Segoe UI Variable Text*, peso semibold (600), 14 DIP; ícones 14 DIP; espaço ícone→texto 4 DIP; entre itens 14 DIP; margem até a bandeja 8 DIP.
- Cores: tema escuro texto `#FFFFFF`, claro `#1A1A1A`; baixo: escuro `#FF6B5E`, claro `#C42B1C`; `--%` com 55% de opacidade.
- Textos da UI em português: menu "Atualizar agora", "Iniciar com o Windows", "Sair"; tooltip `Mouse Zaopin`, `Controle PS5`, `Celular` seguidos de `NN%`, `NN% (carregando)`, `100% (carregado)`, `sem leitura` ou `desconectado`.
- Commits frequentes, mensagens em inglês no estilo `feat: ...`, terminando com `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Jogo em tela cheia** — a faixa é TOPMOST e ficaria por cima do jogo; deve sumir enquanto a janela em primeiro plano cobre o monitor inteiro (Task 6).
2. **`adb` travado** (Wi-Fi caiu no meio do `shell`) — o leitor do celular não pode ficar preso; todo comando `adb` tem timeout de 15 s e é morto depois (Task 3).
3. **Explorer reinicia / barra com auto-ocultar / DPI muda** — faixa reaparece no lugar certo após `TaskbarCreated`, some quando a barra está escondida, e é redesenhada no novo DPI (Task 6).
4. **Barra cheia de apps** — a faixa não pode cobrir botões de apps; se não couber, some (Task 8).
5. **DS4Windows com HidHide ou controle em modo simples `0x01`** — mostra `--%`, nunca trava nem envia nada (Task 2).

---

### Task 1: Projeto + modelo de estado

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/main.rs` (stub), `src/model.rs`

**Interfaces:**
- Produces (`src/model.rs`):
  - `#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum DeviceKind { Mouse, Controller, Phone }` + `pub const ALL: [DeviceKind; 3]` na ordem da faixa.
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Charge { Discharging, Charging, Full }`
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub struct Reading { pub percent: u8, pub charge: Charge }`
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum ReadOutcome { Ok(Reading), Failed, Offline }`
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum ItemView { Value(Reading), Dash, Hidden }`
  - `pub const HOLD: Duration = Duration::from_secs(600);`
  - `pub struct Tracker` com `new() -> Self`, `update(&mut self, kind: DeviceKind, outcome: ReadOutcome, now: Instant)`, `view(&self, kind: DeviceKind, now: Instant) -> ItemView`, `tooltip(&self, now: Instant) -> String`.
  - `pub fn device_label(kind: DeviceKind) -> &'static str` ("Mouse Zaopin", "Controle PS5", "Celular").
  - `pub fn is_low(r: &Reading) -> bool` (`percent < 20`).

Regras de `view` (do spec): último resultado `Ok` → `Value`; senão, se houver leitura boa há ≤ `HOLD` → `Value(essa)`; senão `Failed` → `Dash`; `Offline` ou nada ainda → `Hidden`.
`tooltip`: uma linha por dispositivo em `ALL`, `"<label>: <estado>"` com os textos de Global Constraints (`Hidden` → `desconectado`, `Dash` → `sem leitura`), linhas separadas por `\n`.

- [ ] **Step 1:** `git init` em `D:\dev\battery-bar`; `Cargo.toml` com `name = "battery-bar"`, edition 2024, `[profile.release] opt-level = "s", lto = true, codegen-units = 1, panic = "abort", strip = true`; `.gitignore` com `/target`. `main.rs` com `mod model; fn main() {}`.
- [ ] **Step 2: Testes que falham** em `src/model.rs` (`#[cfg(test)]`), com `let t0 = Instant::now()`:
  - `ok_shows_value`: update `Ok(r 50% Discharging)` → `view == Value(r)`.
  - `failed_after_ok_holds_value`: Ok em t0, Failed em t0+5min → `view(t0+5min) == Value(r)`.
  - `failed_after_hold_shows_dash`: Ok em t0, Failed em t0+11min → `Dash`.
  - `offline_after_hold_hides`: Ok em t0, Offline em t0+11min → `Hidden`.
  - `offline_within_hold_keeps_value`: Ok t0, Offline t0+1min → `Value(r)`.
  - `failed_without_any_good_is_dash` e `nothing_yet_is_hidden`.
  - `tooltip_lists_all`: mouse Failed, controle Ok(64%, Charging), celular nada → `"Mouse Zaopin: sem leitura\nControle PS5: 64% (carregando)\nCelular: desconectado"`.
  - `low_threshold`: `is_low(19%) == true`, `is_low(20%) == false`.
- [ ] **Step 3:** `cargo test` → falha de compilação/asserts.
- [ ] **Step 4:** Implementar `model.rs`.
- [ ] **Step 5:** `cargo test` → todos passam.
- [ ] **Step 6:** Commit (inclua `docs/`): `feat: project scaffold and device state tracker`.

### Task 2: Leitor do DualSense (só escuta)

**Files:**
- Create: `src/readers/mod.rs`, `src/readers/dualsense.rs`
- Modify: `Cargo.toml` (`hidapi = { version = "2", default-features = false, features = ["windows-native"] }` — confirme o nome da feature na doc da versão resolvida)

**Interfaces:**
- Consumes: `Reading`, `Charge`, `ReadOutcome` (Task 1).
- Produces:
  - `pub fn parse_bt_report(buf: &[u8]) -> Option<Reading>` em `dualsense.rs`.
  - `pub fn read_once(api: &mut hidapi::HidApi) -> ReadOutcome` em `dualsense.rs`.
  - `src/readers/mod.rs`: `pub mod dualsense;` (Tasks 3/4 acrescentam os seus).

Formato (driver Linux `hid-playstation`): relatório BT completo tem `buf[0] == 0x31` e ≥ 78 bytes; byte de status em `buf[54]`; `data = status & 0x0F`, `state = status >> 4`. `state 0x0` → Discharging, `0x1` → Charging, percent = `min(data*10 + 5, 100)`; `state 0x2` → Full, 100; qualquer outro → `None`.
`read_once`: `api.refresh_devices()`; procura VID `0x054C`, PID `0x0CE6` ou `0x0DF2`; nenhum → `Offline`. Abre pelo `path` (falha → `Failed`, caso do HidHide). Lê até 20 vezes com `read_timeout(.., 100)`; o primeiro relatório que `parse_bt_report` aceita → `Ok`; se só vier `0x01`/nada → `Failed`. **Nenhuma chamada `write`/`send_feature_report`/`get_feature_report`.** Fecha ao sair (drop).

- [ ] **Step 1: Testes que falham** (`dualsense.rs`), montando `vec![0u8; 78]` com `buf[0]=0x31`:
  - `parses_discharging`: `buf[54]=0x06` → `Some(Reading{65, Discharging})`.
  - `parses_charging`: `0x13` → `Some({35, Charging})`.
  - `parses_full`: `0x2A` → `Some({100, Full})`.
  - `clamps_to_100`: `0x0A` → `Some({100, Discharging})`.
  - `rejects_simple_report`: `buf[0]=0x01` → `None`.
  - `rejects_short`: `&[0x31; 40]` → `None`.
  - `rejects_error_state`: `0xF5` → `None`.
- [ ] **Step 2:** `cargo test dualsense` → falha.
- [ ] **Step 3:** Implementar `parse_bt_report` e `read_once`.
- [ ] **Step 4:** `cargo test dualsense` → passa.
- [ ] **Step 5: Verificação real:** exemplo temporário `examples/ds_once.rs` que imprime `read_once`; `cargo run --example ds_once`. Com Steam aberta e controle ligado espera-se `Ok(...)`; sem Steam/DS4Windows, `Failed`. Anote o resultado no commit; apague o exemplo.
- [ ] **Step 6:** Commit `feat: passive DualSense battery reader`.

### Task 3: Leitor do celular (ADB)

**Files:**
- Create: `src/readers/adb.rs`
- Modify: `src/readers/mod.rs`

**Interfaces:**
- Produces:
  - `pub fn parse_dumpsys(text: &str) -> Option<Reading>`
  - `pub fn parse_ready_serials(text: &str) -> Vec<String>` (linhas de `adb devices` com estado `device`)
  - `pub fn parse_mdns_connect_targets(text: &str) -> Vec<String>` (`host:port` das linhas `_adb-tls-connect._tcp`)
  - `pub fn adb_path() -> PathBuf` (`adb.exe` ao lado do nosso exe se existir, senão `"adb"`)
  - `fn run_adb(args: &[&str]) -> Option<String>`: `Command` com `creation_flags(0x0800_0000)` (CREATE_NO_WINDOW), stdout capturado, `try_wait` em laço com 15 s de limite → `kill()` e `None`; status ≠ 0 → `None`.
  - `pub fn read_once() -> ReadOutcome`

`parse_dumpsys`: `level:` e `scale:` (se `scale` ausente, 100; percent = level*100/scale), `status:` 2 → Charging, 5 → Full, outro → Discharging. Sem `level` → `None`.
`read_once`: `devices` → se vazio, `mdns services` e `connect` em cada alvo, depois `devices` de novo; ainda vazio (ou adb ausente) → `Offline`. Usa o primeiro serial: `-s <serial> shell dumpsys battery` → `parse_dumpsys` → `Ok` / `Failed`.

- [ ] **Step 1: Testes que falham** com fixtures em string:
  - `dumpsys_charging`: texto real (`"Current Battery Service state:\n  AC powered: false\n  USB powered: true\n  status: 2\n  level: 81\n  scale: 100\n"`) → `{81, Charging}`.
  - `dumpsys_full` (`status: 5`, `level: 100`) → `{100, Full}`; `dumpsys_discharging` (`status: 3`, `level: 42`) → `{42, Discharging}`.
  - `dumpsys_garbage` (`"error: device offline"`) → `None`.
  - `devices_filters_states`: `"List of devices attached\n192.168.0.5:37543\tdevice\nabc\tunauthorized\nadb-R5C-xyz._adb-tls-connect._tcp\tdevice\n\n"` → `["192.168.0.5:37543", "adb-R5C-xyz._adb-tls-connect._tcp"]`.
  - `mdns_targets`: `"List of discovered mdns services\nadb-R5C-xyz\t_adb-tls-connect._tcp\t192.168.0.5:37543\nadb-R5C-xyz\t_adb-tls-pairing._tcp\t192.168.0.5:41111\n"` → `["192.168.0.5:37543"]`.
- [ ] **Step 2:** `cargo test adb` → falha.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** `cargo test adb` → passa.
- [ ] **Step 5: Verificação real** (exemplo temporário `examples/adb_once.rs`). Sem celular pareado espera-se `Offline` em < 20 s e nenhuma janela de console piscando. Apague o exemplo.
- [ ] **Step 6:** Commit `feat: adb phone battery reader with timeouts`.

### Task 4: Mouse Zaopin — presença + ferramenta de listagem

**Files:**
- Create: `src/readers/zaopin.rs`, `examples/probe_zaopin.rs`
- Modify: `src/readers/mod.rs`

**Interfaces:**
- Produces: `pub const VID: u16 = 0x3554; pub const PID: u16 = 0xF527;` e `pub fn read_once(api: &mut hidapi::HidApi) -> ReadOutcome` — receptor presente → `Failed` (mostra `--%` até o protocolo existir), ausente → `Offline`. Não abre o dispositivo.

`probe_zaopin`: para cada `DeviceInfo` com esse VID/PID imprime `interface_number`, `usage_page` (hex), `usage` (hex), `path`, e, abrindo o dispositivo, o report descriptor em hex (`get_report_descriptor`). **Nenhuma escrita/feature.** Se abrir falhar (interface de mouse/teclado bloqueada pelo Windows), imprime o erro e segue.

- [ ] **Step 1:** Implementar `zaopin.rs` + exemplo (sem lógica testável além do mapeamento presença→outcome; não escreva teste unitário).
- [ ] **Step 2:** `cargo run --example probe_zaopin` → lista ≥ 1 coleção vendor (`usage_page >= 0xFF00`) de `MI_01`. Salve a saída em `docs/zaopin-probe.txt`.
- [ ] **Step 3:** Commit `feat: zaopin receiver presence and read-only probe`.

### Task 5: Renderização da faixa

**Files:**
- Create: `src/render.rs`
- Modify: `Cargo.toml` (features do `windows`: `Win32_Foundation`, `Win32_Graphics_Direct2D`, `Win32_Graphics_Direct2D_Common`, `Win32_Graphics_DirectWrite`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Gdi`, `Win32_System_Com`, `Win32_UI_WindowsAndMessaging`, `Win32_UI_Shell`, `Win32_UI_HiDpi`, `Win32_System_Registry`, `Win32_System_Threading`, `Win32_UI_Accessibility`, `Win32_Graphics_Dwm` — some as needed nas Tasks 6–8)

**Interfaces:**
- Consumes: `DeviceKind`, `ItemView`, `Reading`, `Charge`, `is_low` (Task 1).
- Produces:
  - `pub struct Strip { pub items: Vec<(DeviceKind, ItemView)>, pub dark: bool }` (itens `Hidden` são ignorados).
  - `pub struct Bitmap { pub width: i32, pub height: i32, pub dib: HBITMAP, pub dc: HDC }` (DIB 32 bpp top-down, BGRA premultiplicado, libera em `Drop`).
  - `pub struct Renderer` com `new() -> windows::core::Result<Self>` (fábricas D2D/DWrite criadas uma vez) e `render(&self, strip: &Strip, dpi: u32, height_px: i32) -> Option<Bitmap>` (`None` se nenhum item visível).
  - `pub fn render_glyph_icon(&self, glyph: char, dark: bool, size_px: i32) -> HICON` (para a bandeja).

Desenho: `ID2D1DCRenderTarget` com `D2D1_ALPHA_MODE_PREMULTIPLIED`, antialias grayscale (ClearType não funciona com alpha). Para cada item: glifo do dispositivo, 4 DIP, texto `NN%` (ou `--%` a 55% de opacidade), e `U+E945` logo após o texto se `Charging`. Cor por `dark`/`is_low` (Global Constraints). Largura = soma medida via DirectWrite; altura = `height_px`; tudo centralizado verticalmente.

- [ ] **Step 1: Testes que falham** (rodam no Windows real, com COM inicializado no teste):
  - `empty_strip_renders_nothing`: só `Hidden` → `None`.
  - `renders_visible_pixels`: um item `Value(64%)` dark, dpi 96, altura 48 → `Some`, alguns pixels com alpha > 0 e cantos com alpha 0.
  - `more_items_is_wider`: 3 itens > 1 item em largura.
  - `scales_with_dpi`: mesma faixa em dpi 144 é ≥ 1,4× mais larga que em 96.
  - `dash_is_dimmer`: maior alpha de `Dash` < maior alpha de `Value`.
- [ ] **Step 2:** `cargo test render` → falha.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** `cargo test render` → passa. (Conferência visual fica para a Task 7; não adicione dependência de PNG.)
- [ ] **Step 5:** Commit `feat: direct2d strip renderer`.

### Task 6: Janela na barra de tarefas

**Files:**
- Create: `src/taskbar.rs`, `src/theme.rs`

**Interfaces:**
- Consumes: `Renderer`, `Strip`, `Bitmap` (Task 5).
- Produces:
  - `pub fn compute_strip_rect(tb: RECT, notify: Option<RECT>, width: i32, height: i32, dpi: u32) -> RECT` — direita = `notify.left - 8 DIP` (sem `notify`: `tb.right - 160 DIP`), esquerda nunca menor que `tb.left + 4 DIP`, verticalmente centralizada em `tb`.
  - `pub fn taskbar_is_dark() -> bool` (`theme.rs`, `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme` == 0; ausente → dark).
  - `pub struct StripWindow` com `create(hinstance) -> Self`, `hwnd(&self) -> HWND`, `set_strip(&mut self, strip: Strip)` (redesenha e reposiciona), `sync(&mut self)` (reposiciona/reafirma TOPMOST/esconde se preciso; chamada por um `SetTimer` de 1000 ms).
  - Mensagens que o dono (Task 7) trata: a wndproc encaminha clique esquerdo/direito como `WM_APP + 2` para a janela-dona passada em `create` (ou expõe callback — escolha do implementador, documente).

Comportamento (spec + Review Focus 1 e 3): estilos `WS_POPUP` + `WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE`; geometria via `FindWindowW("Shell_TrayWnd")` + filho `TrayNotifyWnd` (`GetWindowRect`) + DPI do monitor (`GetDpiForMonitor`); apresenta com `UpdateLayeredWindow` (`ULW_ALPHA`, `AC_SRC_ALPHA`). Esconde quando: barra invisível ou fora da tela (auto-ocultar), faixa vazia, ou a janela em primeiro plano (exceto `Shell_TrayWnd`, `Progman`, `WorkerW` e a própria) cobre o monitor inteiro. Re-renderiza quando o DPI ou a altura da barra mudam, em `RegisterWindowMessageW("TaskbarCreated")` e em `WM_SETTINGCHANGE` com `"ImmersiveColorSet"` (relê o tema). Referência de padrão: `D:\dev\Wireless-Device-Battery-Monitor\src\taskbar.rs` (`query_taskbar_geometry`, `compute_widget_rect`, `sync_widget`, `foreground_is_fullscreen`).

- [ ] **Step 1: Testes que falham** para `compute_strip_rect` (dpi 96, `tb = {0,1032,1920,1080}`):
  - `left_of_tray`: `notify = {1700,1032,1920,1080}`, w 200, h 48 → `{1492,1032,1692,1080}`.
  - `centered_vertically`: h 40 → top 1036.
  - `no_notify_fallback`: `None`, w 200 → right `1760`.
  - `clamped_left`: w 5000 → left `4`.
  - `dpi_150`: dpi 144, notify left 1700 → right `1688`.
- [ ] **Step 2:** `cargo test taskbar` → falha.
- [ ] **Step 3:** Implementar `taskbar.rs` e `theme.rs`.
- [ ] **Step 4:** `cargo test` → passa.
- [ ] **Step 5:** Commit `feat: taskbar strip window`.

### Task 7: Ícone na bandeja, menu, inicialização e ligação geral

**Files:**
- Create: `src/tray.rs`, `src/startup.rs`, `src/app.rs`
- Modify: `src/main.rs`, `src/readers/mod.rs`

**Interfaces:**
- Consumes: tudo acima.
- Produces:
  - `startup.rs`: `pub fn is_enabled() -> bool` (valor `battery-bar` em `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` igual a `"\"<exe atual>\""`), `pub fn set_enabled(on: bool) -> windows::core::Result<()>`.
  - `readers/mod.rs`: `pub fn spawn_all(hwnd_ui: isize, tx: Sender<(DeviceKind, ReadOutcome)>) -> Vec<Sender<()>>` — uma thread por dispositivo (Mouse→zaopin 60 s, Controller→dualsense 60 s, Phone→adb 180 s); cada uma lê, envia, faz `PostMessageW(hwnd_ui, WM_APP + 1, 0, 0)`, e espera `recv_timeout(intervalo)` no seu `Receiver<()>` (mensagem = "atualizar agora"). Cada thread tem seu próprio `HidApi` quando precisa.
  - `tray.rs`: `Shell_NotifyIconW` com ícone de `render_glyph_icon('\u{E83F}', ..)`, tooltip = `Tracker::tooltip` truncado a 127 chars, callback `WM_APP + 3`; menu (`TrackPopupMenu`) com os três itens de Global Constraints, "Iniciar com o Windows" marcado conforme `is_enabled()`. Reinstala o ícone em `TaskbarCreated`.
  - `app.rs`: janela-dona oculta (message-only não recebe broadcasts — use janela top-level invisível), dona do `Tracker`, `StripWindow`, `Renderer`, ícone; em `WM_APP + 1` drena o canal, `tracker.update(..)`, monta `Strip` com `ALL` e `view(now)`, chama `set_strip` e atualiza tooltip. Também reavalia a cada 60 s (expiração do `HOLD`).
  - `main.rs`: `#![windows_subsystem = "windows"]`; `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`; `CoInitializeEx` (apartment); mutex `Local\battery-bar-single` (já existe → sai); loop de mensagens.

- [ ] **Step 1: Testes que falham:** `startup` — `toggle_roundtrip` (liga → `is_enabled()`; desliga → `!is_enabled()`; restaura o estado inicial no fim).
- [ ] **Step 2:** `cargo test startup` → falha. **Step 3:** implementar tudo. **Step 4:** `cargo test` → passa.
- [ ] **Step 5: Verificação manual** (`cargo build --release`, rodar `target\release\battery-bar.exe`):
  - faixa aparece à esquerda da bandeja em ≤ 2 s com `🖱 --%` e `🎮` (valor ou `--%`); celular some;
  - hover no ícone da bandeja mostra 3 linhas; "Atualizar agora" relê; "Sair" encerra e o ícone some;
  - abrir segunda instância → nada acontece;
  - jogo/vídeo em tela cheia → faixa some e volta ao sair;
  - `taskkill /f /im explorer.exe & start explorer` → faixa e ícone voltam;
  - Gerenciador de Tarefas: memória do processo < 10 MB, CPU 0% em repouso.
- [ ] **Step 6:** Commit `feat: tray icon, startup toggle and app wiring`.

### Task 8: Não cobrir botões da barra

**Files:**
- Modify: `src/taskbar.rs`

**Interfaces:**
- Produces: `pub fn occupied_spans(tb_hwnd: HWND, notify: Option<RECT>) -> Vec<(i32, i32)>` (UIA: `IUIAutomation::ElementFromHandle(Shell_TrayWnd)` → `FindAll(TreeScope_Descendants, ControlType == Button)` → `CurrentBoundingRectangle` em x; descarta os que estão dentro de `notify`), e `pub fn overlaps(rect: RECT, spans: &[(i32, i32)]) -> bool`.

A varredura UIA roda numa thread própria a cada 5 s (nunca na thread de UI) e guarda o resultado; `sync` esconde a faixa se `overlaps` for verdadeiro.

- [ ] **Step 1: Testes que falham** para `overlaps`: `{100..200}` vs `[(50,99)]` → false; vs `[(150,160)]` → true; vs `[(200,300)]` → false (borda encostada não conta).
- [ ] **Step 2–4:** falha → implementar → passa.
- [ ] **Step 5: Manual:** abrir muitos apps até os botões chegarem perto da bandeja → faixa some; fechar → volta. CPU continua ~0%.
- [ ] **Step 6:** Commit `feat: hide strip when it would cover taskbar buttons`.

### Task 9: Pareamento do celular + README

**Files:**
- Create: `README.md`

- [ ] **Step 1:** README em português: o que é; como compilar (`cargo build --release`); como parear o celular (Android 11+: Opções do desenvolvedor → Depuração por Wi-Fi → "Parear com código" → `adb pair IP:PORTA` → código; depois o app conecta sozinho via mDNS); como ligar a inicialização; limitações (controle só mostra % com Steam/DS4Windows abertos; mouse `--%` até a captura).
- [ ] **Step 2: Com o usuário:** parear o celular seguindo o README; em ≤ 3 min (ou "Atualizar agora") o item `📱 NN%` aparece e bate com o celular; desligar o Wi-Fi do celular → valor mantido; após 10 min some.
- [ ] **Step 3:** Commit `docs: readme with phone pairing steps`.

### Task 10: Captura do protocolo do Zaopin (com o usuário presente)

Não é implementação fechada: é uma sessão de investigação cujo resultado vira um plano pequeno à parte.

- [ ] **Step 1:** Confirmar com o usuário se o software oficial da Zaopin está instalado e mostra a bateria; se não, ele baixa do site oficial.
- [ ] **Step 2:** Usuário instala USBPcap + Wireshark (com autorização). Capturar no root hub do receptor `3554:F527` enquanto o software abre/atualiza a bateria; anotar o % mostrado.
- [ ] **Step 3:** Identificar o pedido (interface, report ID, bytes) e a resposta cujo byte bate com o %; repetir em dois níveis de bateria diferentes e com o mouse carregando.
- [ ] **Step 4:** Registrar em `docs/zaopin-protocol.md` e escrever um plano curto para trocar `zaopin::read_once` por "enviar exatamente o pedido capturado + `parse_zaopin_reply` com testes das respostas capturadas".

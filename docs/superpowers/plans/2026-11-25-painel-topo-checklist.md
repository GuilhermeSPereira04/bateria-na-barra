# Painel do topo — checklist de teste manual

Nada disto foi testado rodando o programa, porque o battery-bar do usuário estava aberto e o
mouse estava em uso. Antes de testar, feche a instância atual (bandeja → Sair) e rode
`target\release\battery-bar.exe`.

## Abrir e fechar
- [ ] Mouse 1,5 s parado na borda de cima → o painel desce centralizado, com vidro fosco
  escuro e o papel de parede desfocado atrás.
- [ ] Mouse 1 s no topo e depois clicar numa aba do Chrome maximizado → **não** abre.
- [ ] Arrastar uma janela até o topo (aparece o Snap) → **não** abre.
- [ ] Jogo ou vídeo em tela cheia (F11 no YouTube) → **não** abre.
- [ ] Fecha com Esc, com um clique fora (a janela clicada fica com o foco) e com o mouse
  fora por 0,5 s depois de ter entrado.
- [ ] Esc com o mouse ainda no topo, esperando mais de 2 s → **não** reabre. Sair e voltar à
  borda por 1,5 s → abre.
- [ ] Ao abrir, digitar já vai pra busca. Ao fechar com Esc, o foco volta pra janela anterior.
- [ ] Bandeja → "Abrir painel" (primeiro item) abre. Sem passar o mouse por ele, não fecha
  sozinho; Esc e clique fora fecham.
- [ ] Com o painel aberto, clicar na faixa ou na bandeja → o menu aparece e o painel já sumiu.
- [ ] Com uma janela de administrador em primeiro plano, abrir pela borda longe do centro
  → um clique fora fecha, e um clique dentro dá o foco ao painel.
- [ ] Mudar a resolução com o painel aberto → ele fecha. Mudar a escala (100% → 150%) → ele
  se ajusta e continua nítido.

## Aparência e cores
- [ ] A altura acompanha o conteúdo, sem faixa vazia e sem nada cortado. O hover nos botões
  aparece e some.
- [ ] Os cantos são todos arredondados (o Windows não deixa arredondar só os de baixo).
- [ ] ⚙ → trocar a cor e o tom do vidro → muda na hora. Fechar e abrir de novo → continua.
  O arquivo `%APPDATA%\battery-bar\config` existe e dá pra editar à mão.
- [ ] Escolher "Usar a cor do Windows" e trocar a cor no Windows com o painel aberto → o
  painel acompanha.

## Busca de apps
- [ ] `calc` + Enter abre a Calculadora **na frente** e fecha o painel. `code` acha o VS
  Code, com ícone.
- [ ] `camera` acha `Câmera`. `zzz` mostra `Nada encontrado`. As setas ↑/↓ escolhem.
- [ ] Apagar tudo → as abas voltam. Com texto na busca, o mouse fora **não** fecha.
- [ ] Os ícones aparecem sem bordas claras ou escuras e sem quadrado preto, em 100% e 150%.

## Botões rápidos
- [ ] O Wi‑Fi mostra o nome da rede. Clicar desliga/liga (confira no Win+A).
- [ ] O Bluetooth mostra "1 conectado" com o DualSense. Clicar desliga/liga.
- [ ] Seta ou clique direito no Wi‑Fi/BT → Configurações abre **na frente**, na página certa.
- [ ] Roda do mouse no áudio → volume ±2% por clique da roda. Clicar → mudo.
- [ ] Seta do áudio → lista de saídas. Escolher o fone USB → o som vai pra ele. Clique
  direito → Configurações > Som.

## Bateria e player
- [ ] O cartão de bateria mostra o mesmo que a faixa (%, raio, vermelho quando baixo, `--%`).
- [ ] Com Spotify ou YouTube tocando → aparecem título · artista e a capa. ⏯ pausa, ⏭ pula,
  e tudo atualiza em até ~2 s.
- [ ] Sem nada tocando → `Nada tocando` esmaecido.

## Abas
- [ ] Recursos: mostra `…` por ~1 s e depois CPU, GPU e RAM perto dos valores do
  Gerenciador de Tarefas. Os processos de mesmo nome aparecem somados. Clicar em "RAM"
  ordena por RAM.
- [ ] Abrir um jogo → a GPU sobe.
- [ ] Calendário: mês atual com hoje destacado. As setas trocam o mês.
- [ ] Speed test: `Testar` → ping, download e upload perto do speed.cloudflare.com. Durante o
  teste o painel não fecha sozinho. Fechar no meio → a rede volta a ~0. Com o Wi‑Fi
  desligado → `Sem conexão`.

## Consumo (coluna "Memória" do Gerenciador de Tarefas)
- [ ] Anote o valor com o painel fechado logo ao iniciar, com ele aberto em cada aba, e
  fechado de novo depois de 10 ciclos de abrir/fechar passando por todas as abas.
  Meta: fechado ≤ ~8 MB, aberto ≤ ~20 MB.
- [ ] Com o painel fechado ou noutra aba → CPU do battery-bar ~0%.
- [ ] Depois de medir, atualize a linha de consumo do README.

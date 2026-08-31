# web/ — Dashboard do runtime VerboLang

A antiga `scripts/verbo-chat/` vive aqui, agora como a face visual do projeto:
**chat** com o modelo local **e** **métricas em tempo real** do runtime
(Caderno de produção). O LLM é opcional — com o modelo inativo, o estado
aparece identificável no badge e as métricas seguem no ar.

| Arquivo | Papel |
|---|---|
| `index.html` | Entrada do dashboard: cartões para Chat e Métricas + badge do modelo (mesma sonda `/v1/models` do chat) |
| `chat.html` | Chat single-file: streaming SSE, medidor de contexto, alternância "modelo puro ↔ + VerboLang" |
| `metrics.html` | Métricas em tempo real do runtime: energia, sensores, atuações, feed de eventos (SSE) |
| `verbolog.svg` | Marca do projeto (logo + favicon) |
| `fonts/`, `vendor/` | Fontes auto-hospedadas (SIL OFL 1.1) e vendored KaTeX/Mermaid (MIT) |

## Como rodar

```bash
make web                  # só o dashboard + métricas (sem GPU), porta 8188
make serve                # UI na hora + modelo local (a UI não depende do LLM)
make smoke                # confere páginas, vendored e /api/snapshot com o servidor no ar
```

`make serve` sobe **primeiro a UI** e depois o modelo: o dashboard funciona
com o LLM carregando, no ar, sem chave no navegador (401 — o badge distingue)
ou desligado; a UI sobrevive ao Ctrl+C do vLLM (setsid) e vive até
`make stop`.

Servidor: [`scripts/webui.py`](../scripts/webui.py) (só stdlib) — estático na
**raiz do repositório** (o chat busca `/docs/VBL-CHEATSHEET.md`) + rotas de
métricas: `GET /api/sources`, `GET /api/snapshot?src=…`,
`GET /api/events?src=…` (SSE). Bind exclusivo em `127.0.0.1`.

## Métricas em tempo real (como usar)

1. `make web` num terminal.
2. Noutro, execute um programa do runtime apontando o Caderno para dentro do
   repo (o painel lista `logs/**`, `tmp-logs/**` e `caderno*.jsonl`):
   ```bash
   core/target/debug/vbl run examples/example1_free_thinking.vl \
     --ledger tmp-logs/demo.vcad --ticks 5000
   ```
3. Abra `http://127.0.0.1:8188/web/metrics.html`, escolha o ledger no seletor
   (default: o mais recente) e observe o feed chegar ao vivo. Sem rodapé
   `VFIM`, o badge mostra **execução em andamento**; com rodapé, **concluída**.

O que é exibido: Joules acumulados (soma dos `LEAK`), contagem por `kind`,
tick atual, atuações ok/total, última leitura por sensor, último estado por
ator, gráfico de energia por tick e feed filtrável dos últimos eventos.

## Honestidade termodinâmica (limites declarados)

- A ponte **lê** o Caderno (binário `.vcad` — frames `len|linha|hash`, tolera
  frame parcial no flush pendente — ou o `.jsonl` exportado). **Não** verifica
  a cadeia SHA-256: isso é papel do agente externo
  `vbl ledger-verify ARQUIVO` (AGENTS §1.4).
- O Caderno faz flush a cada 256 eventos (PLAN §4.3): as atualizações do
  painel chegam **em lotes** — na cadência da gravação assíncrona, não na do
  evento individual.
- Kinds do vocabulário v1 (PT) são normalizados para o v1.1 (EN), como no
  verificador (`VAZAMENTO→LEAK`, `LEITURA→SENSOR_READ`, …).
- Conexão SSE caiu? O navegador reconecta sozinho e o servidor reenvia o
  snapshot completo (agregados cobrem o histórico; o feed exibe os últimos
  ~300 eventos).

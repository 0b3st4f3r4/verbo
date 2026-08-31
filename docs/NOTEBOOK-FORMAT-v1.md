# NOTEBOOK-FORMAT-v1.md — Formato Binário do Caderno (`.vcad`), v1

**Status:** canônico para a Etapa 4 · **Codec:** [`core/crates/vbl-runtime/src/production_ledger.rs`](../core/crates/vbl-runtime/src/production_ledger.rs) · **Verificador:** `vbl ledger-verify ARQUIVO`

> **Nota de versão — v1.1 (31/08/2026): normalização dos `kinds`.** O
> vocabulário do campo `kind` passou de português para inglês: níveis
> `VAZAMENTO→LEAK`, `LEITURA→SENSOR_READ`, `ALERTA→ALERT`,
> `SUBVERSAO→SUBVERSION`, `ATUACAO→ACTUATION`, `AVALIACAO→ASSESSMENT`,
> `COLAPSO→COLLAPSE`; eventos `transicao→transition`,
> `persistencia→persistence`, `subvert_aplicado→subvert_applied`,
> `keep_forma_inexistente→keep_unknown_form`, `keep_ignorado→keep_ignored`,
> `reclassify_sem_deadline→reclassify_no_deadline`,
> `ator_inexistente→actor_unknown`, `ator_indisponivel→actor_unavailable`,
> `fallback_executado→fallback_executed`,
> `sensor_nao_registrado→sensor_not_registered`,
> `sensor_inacessivel→sensor_inaccessible`. **O verificador aceita para
> sempre os dois vocabulários** — artefatos v1 (PT) permanecem verificáveis e
> produzem estatísticas idênticas (14/14 cadeias históricas conferidas na
> migração). O header (`magic`/versão `0x01`) e a gramática da linha não
> mudam: só o vocabulário do campo; exemplos abaixo já mostram o v1.1.

## 1. Objetivos

| Objetivo (AGENTS §1.4 / PLAN §4.1) | Como o formato atende |
|---|---|
| Log à prova de adulteração | Cadeia SHA-256 incremental (`hash_n = SHA-256(hash_{n-1} ‖ linha_n)`); cada frame carrega o próprio elo |
| Formato binário compacto | Linha canônica UTF-8 + hash cru de 32 B; sem JSON de empacotamento |
| Verificação por agente externo | `vbl ledger-verify` recomputa a cadeia do binário **ou** do JSONL exportado; exit 1 = corrompido |
| Gravação assíncrona sem auto-interferência | Frames independentes append-only (bufwriter + flush a cada 256 eventos) |
| Sobreviver a queda | Rodapé fixo gravado por último: arquivo sem rodapé (execução abortada) ainda verifica até o último frame completo |

**Nota de projeto:** o AGENTS cita Cap'n Proto/FlatBuffers como *exemplos* de
formato binário compacto. Escolhemos um codec zero-dependência na mesma linha
do schema FXP v1 (`docs/FXP-SCHEMA-v1.md`): o "schema" é a própria linha
canônica da cadeia (§3), o que elimina uma segunda representação dos eventos
e mantém o verificador externo trivial (≈ 200 linhas, sem deps).

## 2. Layout do arquivo

```text
┌──────────┬─────────────────────────────────────────────┬──────────────────────────┐
│ header   │ frame₀ frame₁ … frameₙ₋₁                    │ rodapé (72 B, fixo)      │
│ 5 bytes  │ [u32 LE len][linha UTF-8 len B][32 B hash]  │ "VFIM"|eventos u32|head  │
└──────────┴─────────────────────────────────────────────┴──────────────────────────┘
```

### 2.1 Header (5 bytes)

| Campo | Tamanho | Valor |
|---|---|---|
| magic | 4 B | `"VCAD"` (0x56 0x43 0x41 0x44) |
| versão | 1 B | `0x01` |

### 2.2 Frame de evento (repetido n vezes)

| Campo | Tamanho | Conteúdo |
|---|---|---|
| len | 4 B | `u32` little-endian: tamanho em bytes da `linha` |
| linha | len B | linha canônica UTF-8 (§3) |
| hash | 32 B | elo da cadeia, **bytes crus** (o hex de 64 caracteres é só para o JSONL) |

`hash = SHA-256(hash_{n-1} ‖ linha)`, com `hash_{-1} =` 64 caracteres `'0'`
(âncora [`ChainLedger::INITIAL_HEAD`](../core/crates/vbl-runtime/src/ledger.rs)).

### 2.3 Rodapé (72 bytes, tamanho fixo)

| Campo | Tamanho | Conteúdo |
|---|---|---|
| magic | 4 B | `"VFIM"` |
| eventos | 4 B | `u32` little-endiano: quantidade de frames |
| chain_head | 64 B | hash final da cadeia em hex ASCII |

Tamanho fixo de propósito: o verificador lê o fim do arquivo primeiro, confere
`"VFIM"`, trunca o rodapé e percorre os frames — sem ambiguidade de
comprimento. Rodapé ausente (truncagem bruta) ⇒ `rodape_ok = false`, mas os
frames completos continuam verificáveis.

## 3. A linha canônica (fonte única de verdade)

```
linha = seq ␟ kind ␟ msg [ ␟ extra_json ]
```

- separador `␟` = `U+001F` (Unit Separator);
- `extra_json` = objeto JSON com **chaves em ordem de classificação** (`BTreeMap`
  — serializador de [`json.rs`](../core/crates/vbl-runtime/src/json.rs),
  determinístico; inteiros sem casa decimal);
- chaves **reservadas** do `extra`: `tick` e `t` — timestamp do relógio
  virtual (AGENTS §1.4), injetado pelo runtime a cada tick (Etapa 4);
- a MESMA composição é usada pela cadeia em memória (`ChainLedger`), pelo
  binário (frame) e pelo JSONL — um único algoritmo de verificação.

Exemplo (do E2E de subversão térmica):

```
14␟ACTUATION␟Ator 'CpuPowerCap' <- 50 (aplicado: 50, sucesso)␟{"aplicado":50,"ator":"CpuPowerCap","forma":"SpeculativeTrading","sucesso":true,"t":3,"tick":3,"valor":50}
```

## 4. Export JSONL (auditoria textual)

Um objeto JSON por linha, chaves ordenadas: `seq`, `kind`, `msg`, os campos do
`extra` (incluindo `tick`/`t`) **fundidos no nível superior** e `hash` em hex.
O verificador reconstrói a linha canônica separando `seq/kind/msg/hash` do
resto — o JSONL tem a mesma cadeia do binário (testado:
`jsonl_do_binario_reproduz_a_cadeia`).

## 5. Verificação externa

```bash
vbl ledger-verify caderno.vcad            # binário (detecta pelo magic)
vbl ledger-verify caderno.vcad.jsonl      # JSONL
```

Relatório: integridade da cadeia (ÍNTEGRA/CORROMPIDA + primeiro elo
inválido), eventos, cabeça da cadeia, Joules acumulados, atuações (total ×
com sucesso), divergências (alertas de honestidade §4.7) e contagem por kind.
Exit codes: `0` íntegro · `1` cadeia corrompida · `2` erro de leitura/formato.

O `vbl run --ledger ARQUIVO` já verifica o arquivo ao final da execução
(agente externo embutido) e exporta o JSONL irmão (`ARQUIVO.jsonl`).

## 6. Rastreabilidade

| Requisito (fonte) | Cobertura |
|---|---|
| Timestamp do relógio virtual em todo evento (AGENTS §1.4) | chaves `tick`/`t` (§3); teste `engine_com_caderno_de_producao_carimba_relogio_virtual` |
| Gravação assíncrona em buffer, overhead medido (PLAN §4.1/§4.3) | thread dedicada + flush a cada 256; benches `caderno_gravacao`/`caderno_overhead` |
| Formato binário compacto (AGENTS §1.4) | §2; testes de roundtrip/adulteração/truncagem em `tests/production_ledger.rs` |
| Verificação por checksum SHA-256 por agente externo (AGENTS §1.4) | §5; E2E `e2e_caderno_corrompido_falha_o_verificador` |
| Atuações registradas: solicitado/aplicado/latência/custo (PLAN §4.1) | evento `ACTUATION` (v1: `ATUACAO`; campos `valor`, `aplicado`, `latencia_us`, `custo_estimado_joules`); testes de unidade + E2E |
| Métricas agregadas (Joules totais, médias) (AGENTS §1.4) | agregados de `ProductionLedger::fechar()` + relatório do verificador |

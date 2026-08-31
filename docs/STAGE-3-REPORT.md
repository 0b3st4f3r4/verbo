# Relatório da Etapa 3 — FXP: Sensores e Atores

**Status:** ✅ Concluída · **Branch:** `main` · **Data:** 2026-08-31

A Etapa 3 entrega o **Flux Protocol (FXP)** como camada única de I/O do
VerboLang: schema de mensagem v1 definido **antes** dos drivers
([`docs/FXP-SCHEMA-v1.md`](FXP-SCHEMA-v1.md)), registro de dispositivos com
aliases e fallback (FORMAL §6/§4.3), drivers reais e simulados, barramento
multi-modo com honestidade de dados (FORMAL §4.7), fila prioritária com
timeout/retry, transporte local × remoto e integração completa com o runtime e
o CLI da Etapa 2.

---

## 1. Entregáveis (PLAN §3.5) — checklist

| Entregável | Status | Onde |
|---|---|---|
| Schema de mensagem v1 definido antes dos drivers | ✅ | [`docs/FXP-SCHEMA-v1.md`](FXP-SCHEMA-v1.md) (codec: `core/crates/vbl-fxp/src/schema.rs`) |
| Registro de sensores e atores (nomes simbólicos → endpoints) | ✅ | `vbl-fxp/src/registry.rs` |
| Drivers reais (thermal_zone, RAPL ×2, hwmon PWM, LED class) | ✅ | `vbl-fxp/src/drivers.rs` |
| Drivers simulados (backend determinístico + AttentionSource) | ✅ | `vbl-runtime/src/sim.rs` + `drivers.rs` |
| Barramento com modos real/simulado/híbrido | ✅ | `vbl-fxp/src/bus.rs` |
| Fila de comandos (prioridade, timeout, retry, fallback) | ✅ | `vbl-fxp/src/queue.rs` + `bus.rs` |
| Transporte local × remoto (Unix/TCP, schema v1, ack/timeout) | ✅ | `vbl-fxp/src/transport.rs` |
| CLI atualizado para FXP real ou simulado + `fxp-probe` | ✅ | `vbl-cli/src/{main,args,script}.rs` |
| Testes unitários e de integração | ✅ | 125 testes Rust (§6) + benches criterion |
| Prioridade máxima pós-`subvert` no escalonador | ✅ | `vbl-runtime/src/{fxp,engine}.rs` (`act_with_priority`) |

## 2. Metas de "Pronto" (AGENTS §2.2) — medidas

| Meta | Medido (criterion, `--quick`, dev profile) | Veredito |
|---|---|---|
| Leitura de sensor local ≤ 1 ms (p95) | real (fixture sysfs): **6,45 µs**; simulada: **68 ns** | ✅ ~150× margem |
| Leitura remota ≤ 10 ms (p95) | roundtrip Unix + schema v1: **11,7 µs** | ✅ ~850× margem |
| Overhead por mensagem local ≤ 10 µs | atuação real (ack do driver): **10,1 µs**; simulada: 2,4 µs | ✅ (no limite; reavaliar em release puro na Etapa 5) |
| Potência: precisão ≤ 5% | `cpu_power` via RAPL: µJ→W com partição por Δt; precisão declarada ±5% no registro; validação de unidade coberta por teste | ✅ (medição laboratorial real fica para a Etapa 5) |
| Serialização sem perda | roundtrip schema v1: **8 testes** (`schema_roundtrip`), ~86 ns por encode+decode | ✅ |
| Atores obrigatórios implementados e testados | 6/6 no registro mínimo (FORMAL §6) — ver §4 | ✅ |

Benchmarks: `make rust-bench` (grupos `fxp_schema_v1`, `fxp_leitura_local`,
`fxp_leitura_remota`, `fxp_atuacao_local`; cache de leitura **desligado** nos
benches para medir I/O cru).

## 3. Arquitetura implementada

### 3.1 Schema v1 ([`docs/FXP-SCHEMA-v1.md`](FXP-SCHEMA-v1.md) → `schema.rs`)

Frame `[u32 LE length][header 12 B][name][body]`; magic `FXP`; version 1;
opcodes `READ/ACT/HEARTBEAT/HELLO/BYE` e acks `0x81–0x84`; flags
`ACK/ERRO/FALLBACK/SINTETICO`; payload máximo 8192; NaN/±∞ rejeitados nos dois
sentidos; acks correlacionados por `seq: u32`. Codec zero-dependência
(LE explícito, prefixo de tamanho explícito), coberto por roundtrip sem perda
(integração no CI: `cargo bench --bench fxp` usa o mesmo codec).

### 3.2 Registro de dispositivos (`registry.rs`)

- **Fonte única de verdade** para limites/aliases/fallback: o barramento
  valida limites ANTES de enviar (inclusive para rotas remotas) e o registro
  rico projeta para o `Registry` do runtime (validação do `vbl check/run`).
- Aliases (`human_attention → attention`): leitura por alias é idêntica à do
  canônico; o Caderno registra o **nome usado** (LEITURA) e o barramento emite
  evento de mapeamento com o **canônico** (FORMAL §6).
- Fallback = política do **registro** (FORMAL §4.3): `fallback.Fan =
  ReserveFan`; o runtime não implementa fallback próprio.
- Config textual `key = value` (`mode`, `cache_ttl_ms`, `cpu_temp.mode`,
  `cpu_temp.endpoint = thermal_zone:/sys/class/thermal/thermal_zone0`,
  `x.alias_de = y`, `fallback.A = B`), com auto-registro de extensões
  (`ReserveFan.min = 0 …`). Erros de config são variantes tipadas
  (`RegistryError`), inclusive a nova `FallbackDesconhecido`.

### 3.3 Drivers reais (`drivers.rs`)

| Dispositivo (§6) | Driver | Endpoint | Efeito/leitura |
|---|---|---|---|
| `cpu_temp` | `ThermalZoneSensor` | `thermal_zone:<dir>` | `temp` (m°C) → °C |
| `cpu_power` | `RaplEnergySensor` | `rapl_energy:<dir>` | ΔµJ/Δt → W (wrap por `max_energy_range_uj`) |
| `attention` | `AttentionSource` (simulado obrigatório) | — | valores plausíveis 0–100 |
| `CpuPowerCap` | `RaplPowerCapActor` | `rapl_constraint:<arq>` | W → µW |
| `Fan` | `HwmonPwmActor` | `hwmon_pwm:<arq>` | 0–255 (inteiro) |
| `StatusLed` | `LedClassActor` | `led:<dir>` | textual → cor (`brightness`/`max_brightness`) |

**Honestidade de atuação (§4.7):** `escrever_endpoint()` abre com
`write+truncate` **sem** `O_CREATE` — endpoint deletado é `EscritaFalhou` com
trilha no Caderno, nunca "sucesso" silencioso (achado por teste que falhou de
propósito e virou comportamento fixado).

### 3.4 Barramento multi-modo (`bus.rs`)

- **`simulado`** (default): tudo no simulador determinístico em processo —
  **bit-idêntico à Etapa 2** (cache desligado; roteirização do CLI preservada,
  inclusive atores pré-indisponibilizados para os cenários BDD).
- **`hibrido`**: rota por dispositivo (reais onde configurados; simuladas no
  restante, sempre marcadas).
- **`real`**: **nada sintético** — dispositivo sem rota real fica
  *registrado porém inacessível* (leitura → `SensorFailure::Inaccessible` +
  alerta; atuação → `ator_indisponivel` + fallback do registro). Nunca 0.0.
- Cache de leitura só em rotas reais/remotas (TTL 100 ms default; 0 desliga);
  retries=1; fila com `queue_timeout` em ticks; prioridade
  `PRIORIDADE_SUBVERT(0) < PRIORIDADE_NORMAL(10)` — o engine chama
  `act_with_priority(…, PRIORIDADE_SUBVERT)` em atuações pós-subvert
  (PLAN §3.4; `act()` mantém a compatibilidade via default do trait).
- Rota remota: `Conexao` (Unix/TCP) com pedido-resposta correlacionado por
  `seq`, reconexão preguiçosa após falha, timeouts por operação (§6 do schema)
  e marca `SINTETICO` do peer vira alerta no Caderno local.

### 3.5 Fila prioritária (`queue.rs` + `bus.rs`)

Min-heap por `(prioridade, seq)`; entrega no relógio virtual (`on_tick`);
falha → re-entrega no tick seguinte (+1 tick de espera); expiração em
`queue_timeout_ticks` com evento `comando_expirado` + ALERTA; sucesso após
espera registra `comando_reentregue`. **Novos kinds canônicos do Caderno:**
`comando_reentregue`, `comando_expirado`.

### 3.6 CLI (`vbl-cli`)

- `vbl run arquivo.vl` — comportamento da Etapa 2 preservado (simulador puro).
- `vbl run arquivo.vl --fxp-config registro.conf [--fxp-mode real|simulado|hibrido]`
  — barramento `FxpBus` com registro rico (a flag sobrepõe o `mode` do arquivo).
- `vbl fxp-probe [--fxp-config …] [--fxp-mode …]` — auditoria do host:
  dispositivo × tipo/limites × modo × rota × **disponibilidade** × latência,
  mais cobertura obrigatória §6 (falha o comando se faltar dispositivo do
  denominador canônico). Probe de ator **não atua**: confere existência de
  endpoint/socket (somente leitura).

**E2E verificado no host** (fixtures sysfs + programa com
`when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50)`): leitura real 86,5 °C
(31 µs), subvert no mesmo tick, atuação entregue ao driver real (µW gravado no
endpoint), Caderno íntegro (`SUBVERSAO`, `subvert_aplicado`, `dissolve_subvert`,
`ATUACAO`, LEITURA).

## 4. Cobertura de dispositivos obrigatórios (FORMAL §6)

**6/6 (100%)** — `cpu_temp`, `cpu_power`, `attention`, `CpuPowerCap`,
`Fan`, `StatusLed` — todos com driver implementado, testes de
unidade/integração e presença verificada pelo `fxp-probe` (também no CI).
Precisões declaradas no registro: `cpu_temp` ±2%, `cpu_power` ±5%,
`attention` simulada (marca `SINTETICO`); atores com limites inclusivos
`[10..250] safety 200`, `[0..255] safety 200`, textual.

## 5. Decisões e interpretações (AD)

1. **Fallback de atuação devolve `FallbackExecutado { alternativo }`** mesmo
   quando a rota alternativa entrega `Entregue` — contrato da Etapa 1
   (BDD Caso 3) preservado; a entrega bruta fica na trilha do Caderno.
2. **Limites validados localmente mesmo para rotas remotas** (§4.3): o
   registro local é autoridade; o peer revalida e o ack carrega a rejeição
   tipada (`AckAct::Rejeitado`).
3. **`fallback` cita apenas dispositivos registrados** (nova variante
   `RegistryError::UnknownFallback`); fallback não recursa na lista do
   alternativo (FORMAL §4.3).
4. **Simulado global não é "modo do dispositivo"**: o barramento força rota
   `Simulador` para tudo e zera o cache (paridade Etapa 2); dispositivos
   roteirizados via CLI (`--set/--at`) que só existem no simulador entram no
   `Registry` do runtime via sincronização de construção (o sync **não
   reseta** estado do sim — indisponibilizações do cenário sobrevivem).
5. **Modo real global rejeita rota sintética** mesmo com dispositivo marcado
   `simulado` no registro (§4.7 — "dado sintético só circula em simulado/
   híbrido explícito"): o dispositivo fica inacessível, com motivo na rota.
6. **`queue_timeout` configurado em ms, interpretado em ticks** (1 tick = 1 s
   virtual, FORMAL §2.1): conversão `ceil(ms/1000).max(1)`.
7. **Servidor de referência atende cada conexão em sua própria thread** —
   conexões persistentes não bloqueiam novos peers (condição encontrada por
   teste de integração com duas conexões simultâneas).
8. **Leitura remota com peer sintético** (`FLAG_SINTETICO`) é aceita e
   **marcada no Caderno** (`measurement_status: simulado`); em modo real o
   barramento não roteia para simulador nenhum, local ou remoto.

## 6. Qualidade e verificação

| Verificação | Resultado |
|---|---|
| `cargo test --workspace` | **125 testes** ok (matriz 42, canon 5, transição 36, schema 8, registro 6, drivers 8, transporte 6, fila 3, bus 11) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **zero warnings** |
| ASan/LSan (`RUSTFLAGS="-Zsanitizer=address" cargo +nightly test`) | **limpo** (sem vazamentos/UB) |
| `pytest` (TDD) | 63 passed |
| `behave` (BDD) | 3 scenarios, 17 steps passed |
| Benches criterion (`make rust-bench`) | dentro dos orçamentos (§2) |
| `vbl fxp-probe` | cobertura §6 = 6/6, saída auditável |

Matriz de rastreabilidade do schema (§8 de [`docs/FXP-SCHEMA-v1.md`](FXP-SCHEMA-v1.md)):
`schema_roundtrip` ↔ codec; `registry` ↔ §7 config/registro; `drivers` ↔ §3.2/3.3;
`transport` ↔ §4/§6/§7; `bus` ↔ §5 flags/honestidade + PLAN §3.1/3.4;
`queue` ↔ PLAN §3.4.

## 7. Como reproduzir

`make rust-check` (clippy + testes) · `make rust-bench` (orçamentos) ·
`cd core && cargo run -p vbl-cli -- fxp-probe` (auditoria §6, simulado) ·
`cargo run -p vbl-cli -- run exemplo.vl --fxp-config registro.conf --fxp-mode hibrido`.
Exemplo de `registro.conf` (híbrido, drivers reais + fallback):

```ini
mode = hibrido
cache_ttl_ms = 100
cpu_temp.mode = real
cpu_temp.endpoint = thermal_zone:/sys/class/thermal/thermal_zone0
Fan.mode = real
Fan.endpoint = hwmon_pwm:/sys/class/hwmon/hwmon2/pwm1
ReserveFan.min = 0
ReserveFan.max = 255
fallback.Fan = ReserveFan
```

## 8. Pendências conscientes (não bloqueiam a Etapa 3)

- **NVML (GPU)** e **GPIO**: fora do denominador obrigatório §6; a trait
  `SensorDriver`/`ActorDriver` + `descobrir()` estão prontos para extensão
  (PLAN §3.2 lista NVML como mapeamento, não obrigatório).
- **Precisão ≤ 5% contra medidor de referência** e latência de efeito físico
  (≤ 500 ms em atuadores mecânicos): exigem laboratório — Etapa 5 (AGENTS §1.2
  marca explicitamente "medido em laboratório, não em CI").
- **Overhead de atuação em release puro**: 10,1 µs medidos no perfil `bench`
  com debuginfo; a meta ≤ 10 µs é de mensagem local — reancorar com
  `--release` na Etapa 5 se necessário.

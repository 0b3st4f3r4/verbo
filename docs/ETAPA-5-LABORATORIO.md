# ETAPA-5-LABORATORIO.md — Validação laboratorial (Etapa 5, PLAN §5.1)

**Data:** 31/08/2026 · **Host:** AMD Ryzen 7 7735HS (Zen3+, 16 threads), Linux
7.0.0-29 · **Rota:** FXP em modo **híbrido** com dispositivos reais + soak de
24 h em execução simultânea (PID registrado em `/tmp/vbl-soak-24h.pid`).

Esta sessão fecha as pendências laboratoriais registradas na Etapa 5
([ETAPA-5-RELATORIO.md](ETAPA-5-RELATORIO.md) §5–§6): execução longa de 24 h,
energia com leitura RAPL real, perf fino e validação do FXP em hardware real.

---

## 1. FXP híbrido em hardware real — registro e cenários

Registro do laboratório: [`logs/etapa5/lab/fxp-lab.cfg`](../logs/etapa5/lab/fxp-lab.cfg)
(`mode = hibrido`). `vbl fxp-probe` com a config:

| Dispositivo | Rota | Disponibilidade |
|---|---|---|
| `cpu_temp` | **real** — `hwmon_temp:/sys/class/hwmon/hwmon4/temp1_input` (k10temp) | ✓ 87,4 °C (25 µs) |
| `cpu_power` | **real** — `rapl_energy:/sys/devices/virtual/powercap/intel-rapl/intel-rapl:0` | ✓ após chmod do `energy_uj` |
| `CpuPowerCap` | **real** — `rapl_constraint:…/intel-rapl:0:0/constraint_0_power_limit_uw` | ✗ endpoint **ausente** (host não expõe constraint) |
| `Ventoinha`, `LedIndicador`, `attention` | simulado | ✓ (fan sem pwm exportado; attention exige backend próprio, §6) |

### 1.1 Subversão térmica disparada por temperatura FÍSICA (BDD Caso 2 real)

`logs/etapa5/lab/hw-subversao-termica.vcad` (+ `.jsonl`, cadeia ÍNTEGRA) — o
cenario canônico (`exemplo2_trading_especulativo.vl`) com o sensor real:
k10temp acima do limiar de 85 °C **disparou a revisão de verdade**:

```
LEITURA   Sensor 'cpu_temp' = 87.5            (k10temp, hardware real)
ALERTA    Condição disparada: cpu_temp > 85 (lido: 87.5)
SUBVERSAO Operador subvert() invocado! Acumulação abortada.
ATUACAO   Ator 'CpuPowerCap' <- 50 (4 µs, falha)
ALERTA    Todos os fallbacks de 'CpuPowerCap' falharam.
INFO      ALÍVIO TERMODINÂMICO -> Forma dissolvida (mesmo tick).
```

- A atuação falha é a **rota §4.7 honesta**: endpoint ausente ⇒ alerta no
  Caderno, comando não enviado, nenhuma fabricação de sucesso.
- Contraste com ator acessível: `hw-subversao-ventoinha.vcad` — mesmo disparo
  real, `ATUACAO Ator 'Ventoinha' <- 200 (aplicado: 200, sucesso)`,
  `atuações 1/1 ok`.
- Log-bandeira com física real: `hw-subversao-completa.vcad` — forma viva por
  2 ticks com partilha da potência RAPL real (38,49 J + 35,78 J), disparo
  roteirizado (`--at 3:attention=15`), SUBVERSAO com acumulação abortada,
  atuação com sucesso — **74,27 J acumulados, cadeia ÍNTEGRA**.

## 2. Extensão do FXP: endpoint `hwmon_temp` (diretório dinâmico §6)

O k10temp não é exposto como `thermal_zone` (só `acpitz`, placa-mãe) — o
endpoint `hwmon_temp:<arquivo>` foi adicionado (`registry.rs`, `drivers.rs`),
com testes de conversão m°C→°C, falha honesta (§4.7) e parse round-trip.

## 3. Dois bugs reais encontrados pelo laboratório (corrigidos com testes)

1. **`relogio_parede` medindo ~0**: `Instant::now().elapsed()` sobre um
   instante **recém-criado** — todo Δt saía em nanosegundos e `W = ΔE/Δt`
   fabricava potências absurdas (823 MW medidos no primeiro experimento).
   Correção: base capturada uma única vez; regressão
   `relogio_parede_avanca`.
2. **Par degenerado sobrescrevendo a partilha**: o mesmo driver é lido mais
   de uma vez por tick (auditoria × avaliação); a re-leitura, µs depois, com
   ΔE de um quantum e Δt de µs, sobrescrevia a última média válida. Correção:
   par com Δt < 1 ms não informa potência (mantém a amostra anterior válida —
   a próxima cobre a janela inteira); regressão
   `rapl_energy_par_degenerado_nao_corrompe_potencia`.

Ambos são exatamente o caso de uso da validação em hardware: latentes em
simulado, imediatos no mundo físico. Suíte: **149 testes** verdes
(+2 regressões), clippy `-D warnings` limpo.

## 4. Precisão energética Caderno × RAPL (AGENTS §1.4)

Protocolo ([`rapl-experimento.sh`](../logs/etapa5/lab/rapl-experimento.sh),
saída em [`rapl-precisao.txt`](../logs/etapa5/lab/rapl-precisao.txt)): 1 janela
de repouso (60 s; piso 36,03 W com soak de fundo + desktop) e 3 janelas de
carga — 92 ticks × 1 s de parede (`--real-ms 1000`), forma única recebendo a
partilha integral da potência RAPL lida a cada tick. Comparação **full-to-full**:
o runtime atribui a potência integral do package, então
`Σ E_caderno` deve igualar `Δenergy_uj` da mesma janela.

| Janela | E_caderno | E_RAPL | Erro relativo | P média |
|---|---|---|---|---|
| carga 1 | 3 260,70 J | 3 261,18 J | **−0,0146 %** | 35,8 W |
| carga 2 | 3 264,52 J | 3 265,13 J | **−0,0186 %** | 35,9 W |
| carga 3 | 3 239,80 J | 3 240,28 J | **−0,0147 %** | 35,6 W |

Orçamento do AGENTS §1.4: sensor ±5 % + método 1 % ⇒ **atendido com ~2 ordens
de grandeza de folga**. O viés negativo constante (~−0,015 %) é o tick de
aquecimento do sensor RAPL (primeira amostra não atribui) mais o consumo do
próprio processo nas bordas da janela — ambas as parcelas declaradas.
**Escopo honesto:** isto valida a *contabilidade de partilha* (integral da
potência lida contra o mesmo contador RAPL), não a precisão do sensor contra
um medidor externo independente — essa exigiria wattímetro de referência.

## 5. Perf fino (caminhos quentes)

`perf_event_paranoid=1` aplicado; `perf record -F 99 -g --call-graph dwarf`
acoplado ao soak de 24 h (30 s, 2 993 amostras) e a uma instância idêntica
curta (45 s, 4 470 amostras, símbolos completos —
[`perf-soak2-report.txt`](../logs/etapa5/lab/perf-soak2-report.txt)).
Achado principal: `__memcmp_avx2_movbe` domina (~28 %) — comparação de strings
nas buscas `BTreeMap`/`ordem.retain`/heap do escalonador, coerente com a
análise da dissolução O(N) registrada em
[ETAPA-5-RELATORIO.md](ETAPA-5-RELATORIO.md) §5; `memmove` (~6 %) e
malloc/free (~3 %) completam o perfil de mutação estrutural por tick.
Símbolos do binário antigo aparecem como "(deleted)" no acoplamento ao soak
de 24 h (o binário foi substituído pelo rebuild das correções — o processo
roda o inode original, função idêntica).

## 6. Execução longa (24 h)

Soak em andamento — `logs/etapa5/soak-24h.log` (PID em
`/tmp/vbl-soak-24h.pid`, sessão própria via `setsid`: sobrevive a reinícios
do harness). Conclusão registrada ao término (janela de ~24 h a partir de
31/08 ~07:45 local). Estado inicial: patamar 3 200 KiB; amostras a cada
50 000 ticks (~3 min).

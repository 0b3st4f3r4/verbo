# Instalação e primeiro run

**Neste capítulo:** instalar o interpretador `vbl` (duas rotas: crates.io ou
direto do código), validar um programa, rodar a subversão térmica com FXP
simulado e conferir a integridade do Caderno. Nada aqui exige hardware real —
o FXP simulado é completo o bastante para aprender.

## Pré-requisitos

- **Rust stable** (o núcleo é Rust; piso declarado: 1.87) via
  [rustup](https://rustup.rs);
- opcional para estudo offline: Python 3.10+ (o protótipo de referência) e
  GNU make.

## Duas rotas para o `vbl`

**Rota crates.io** (a partir da release
[v2027.0.0-alpha.0](https://crates.io/crates/vbl-cli)):

```bash
cargo install --locked vbl-cli
vbl --help
```

**Rota do código-fonte** (sempre funciona; é a rota dos que vão mexer no
núcleo):

```bash
git clone https://github.com/0b3st4f3r4/verbo.git
cd verbo
cargo install --locked --path core/crates/vbl-cli
```

> [!NOTE]
> Sem instalar nada, também é possível **compilar e rodar do workspace**:
> `cargo build -p vbl-cli` em `core/` produz `core/target/debug/vbl`. Nos
> exemplos abaixo, `vbl` é o binário instalado; troque pelo caminho do
> target se preferir.

Verifique:

```bash
vbl check examples/example2_speculative_trading.vl
# ok: examples/example2_speculative_trading.vl — programa válido
```

## O que o compilador rejeita

O parser cobra o contrato físico antes de qualquer execução. Uma forma sem
`horizon` — existência sem prazo — é **erro de compilação**, não aviso:

```text
$ vbl check quebrado.vl
1:1 [erro] horizon_obrigatorio: forma 'SemHorizonte' sem 'horizon' —
           obrigatório em toda conjugação (Lei 1)
1:1 [erro] maintenance_deadline_ausente: forma 'SemHorizonte':
           nonequilibrium exige maintenance_deadline — sem ele a forma
           jamais colapsaria
tmp-logs/quebrado.vl: 2 erro(s) de compilação
```

Outras cláusulas de erro que você vai encontrar cedo: atributo repetido na
mesma forma, vírgula depois do último atributo, `cost_bytes` fora de
`equilibrium`, `maintenance_deadline` fora de `nonequilibrium`, `review`
órfã ou duplicada, `keep` fora do `main`, identificadores com acento. A
lista completa com justificativa está na
[especificação formal, §3](../../../docs/FORMAL.md).

## Primeiro run: a subversão térmica

O exemplo a seguir roda o programa canônico com o **FXP simulado**: começa
com a CPU a 90 °C e deixa o runtime agir.

```bash
vbl run examples/example2_speculative_trading.vl \
    --ticks 8 --set cpu_temp=90 --ledger tmp-logs/demo.vcad
```

Saída (resumida):

```text
▶ examples/example2_speculative_trading.vl — relógio virtual 1 tick = 1s
  Caderno de produção: tmp-logs/demo.vcad (assíncrono; JSONL em …jsonl)
  1 forma(s) carregada(s)
■ 8 tick(s) — formas ativas restantes: —
  cadeia SHA-256 ÍNTEGRA: 11 evento(s); atuações 1/1 ok
  cabeça da cadeia: 8257faf7827bdce6…
```

O que aconteceu, tick a tick:

1. **Tick 1** — o runtime lê `cpu_temp` no FXP (90, simulado) e registra a
   leitura e o vazamento energético (150 W × 1 s = 150 J, repartidos entre
   as formas ativas);
2. a regra `when cpu_temp > 85°C` dispara: `subvert` substitui o valor pelo
   poético canônico e **dissolve a forma no mesmo tick**;
3. o `act(CpuPowerCap, 50)` da mesma regra **não é cancelado** — o comando
   sai para o ator, que aceita (50 está dentro dos limites [10, 250]);
4. o Caderno grava a cadeia completa: leitura, alerta, subversão, atuação,
   dissolução (`dissolve_subvert`), alívio termodinâmico.

A auditoria é **externa** ao runtime — você, um script de CI ou um
processo separado:

```bash
vbl ledger-verify tmp-logs/demo.vcad
# cadeia SHA-256 ÍNTEGRA: 11 evento(s) no arquivo; atuações 1/1 ok
```

> [!TIP]
> O JSONL exportado (`demo.vcad.jsonl`) é o mesmo log em texto — útil para
> `jq` e para estudar o vocabulário de eventos
> ([capítulo 6](caderno.md)). Para ver o Caderno **ao vivo** num painel,
> rode `make web` e abra `web/metrics.html` apontando para o arquivo.

## Brincando com o tempo

O relógio virtual é dirigido pela CLI — determinismo total, sem dormir:

```bash
# fadiga de atenção: começa em 100% e cai para 10% no tick 4
vbl run examples/example1_free_thinking.vl --ticks 12 \
    --set attention=100 --at 4:attention=10 \
    --ledger tmp-logs/atencao.vcad

# bloco main: keep() a cada 4s, LED a cada 10s
vbl run examples/example5_main_task.vl --ticks 30 \
    --ledger tmp-logs/main.vcad
```

Flags úteis nesta fase:

| Flag | Para quê |
|---|---|
| `--ticks N` | quantos ticks virtuais rodar (padrão: até o mundo esvaziar) |
| `--set SENSOR=V` | valor inicial de um sensor no FXP simulado |
| `--at TICK:SENSOR=V` | roteira um valor absoluto num tick futuro |
| `--ledger ARQUIVO` | grava o Caderno de produção (`.vcad` + `.jsonl`) |
| `--fxp-mode MODO` | `simulado` (padrão) · `real` · `hibrido` |
| `--register-actor NOME` | registra um ator extra 0..255 no simulado |
| `--allow-unregistered` | roda mesmo com sensor/ator fora do registro (§4.7 — condição deixa de ser avaliada) |

## O protótipo Python (opcional)

Antes do núcleo Rust, a linguagem tem um protótipo de referência em Python
puro — emulador de engine, FXP e Caderno:

```bash
python3 prototype/verbolang-complete-blueprint.py
```

Ele existe como *especificação executável*: quando a FORMAL e o Rust
divergirem em dúvida, o blueprint documenta a intenção original.

## Próximo passo

[As três conjugações](formas.md): `event`, `equilibrium` e
`nonequilibrium` — os três modos de existir e o que cada um custa.

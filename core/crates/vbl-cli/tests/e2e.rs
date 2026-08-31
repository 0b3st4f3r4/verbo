//! E2E da Etapa 4 (PLAN §4.2): o interpretador integrado (CLI `vbl` + FXP +
//! Caderno de produção) submetido aos cenários comportamentais da Etapa 1.
//!
//! Cada cenário roda o binário de verdade, exporta o log do Caderno
//! (binário + JSONL), verifica a integridade da cadeia SHA-256 com o
//! verificador externo (`vbl ledger-verify`) e audita as atuações
//! registradas (valor solicitado/aplicado, latência, custo) — critérios de
//! "Pronto" da Etapa 4 (AGENTS §2.2).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Diretório por cenário (isolado por PID; limpeza best-effort no fim).
fn scenario(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vbl-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Executa o `vbl` com os argumentos dados (E2E real: processo separado).
fn vbl(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vbl"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("executar vbl")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(dir: &Path, name: &str, content: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path.display().to_string()
}

/// Linha de comando completa: programa + extras + persistência + Caderno.
fn args_run(dir: &Path, program: &str, ledger: &str, extras: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["run", program].iter().map(|s| s.to_string()).collect();
    for e in extras {
        args.push(e.to_string());
    }
    args.push("--persist-dir".into());
    args.push(dir.join("persistence").display().to_string());
    args.push("--ledger".into());
    args.push(dir.join(ledger).display().to_string());
    args
}

fn run(dir: &Path, args: &[String]) -> (Output, String) {
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = vbl(dir, &refs);
    let text = stdout(&out);
    (out, text)
}

/// Verificação externa do Caderno do cenário (deve sair ÍNTEGRA).
fn verify_external(dir: &Path, ledger: &str) -> String {
    let verify = vbl(dir, &["ledger-verify", dir.join(ledger).to_str().unwrap()]);
    let report = stdout(&verify);
    assert!(verify.status.success(), "ledger-verify falhou:\n{report}");
    assert!(report.contains("ÍNTEGRA"), "cadeia corrompida:\n{report}");
    report
}

fn clear(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

// ======================================================================
// BDD Caso 2 (PLAN §1.1): subversão poética por sobrecarga térmica
// ======================================================================

#[test]
fn e2e_thermal_subversion_acts_on_actor_and_audits() {
    let dir = scenario("subversao");
    let program = write(
        &dir,
        "trading.vl",
        "nonequilibrium SpeculativeTrading {\n\
         \x20 value: \"lucro_arbitragem_alta_frequencia\",\n\
         \x20 horizon: 7s,\n\
         \x20 source_path: \"cpu_temp\",\n\
         \x20 maintenance_deadline: 2s,\n\
         \x20 exchange_mode: \"extraction\"\n\
         }\n\
         review SpeculativeTrading {\n\
         \x20 when cpu_temp > 85°C -> subvert,\n\
         \x20                         act(CpuPowerCap, 50)\n\
         }",
    );
    let (out, text) = run(
        &dir,
        &args_run(&dir, &program, "caderno.vcad", &["--ticks", "5", "--at", "3:cpu_temp=86.5"]),
    );
    assert!(out.status.success(), "vbl run falhou:\n{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("ÍNTEGRA"), "cadeia não íntegra:\n{text}");
    assert!(text.contains("atuações 1/1 ok"), "atuação não confirmada:\n{text}");

    let report = verify_external(&dir, "caderno.vcad");
    assert!(report.contains("ACTUATION: 1"), "atuação não registrada:\n{report}");

    // atuação auditada: solicitado = aplicado = 50, no tick da condição
    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    let actuation = jsonl.lines().find(|l| l.contains("\"kind\":\"ACTUATION\"")).expect("ATUACAO no log");
    for expected in [
        "\"ator\":\"CpuPowerCap\"",
        "\"valor\":50",
        "\"aplicado\":50",
        "\"sucesso\":true",
        "\"tick\":3",
        "\"t\":3",
    ] {
        assert!(actuation.contains(expected), "ATUACAO sem {expected}: {actuation}");
    }
    // no modo SIMULADO a fronteira é em processo: sem latência física ⇒ sem
    // custo estimado (honestidade §4.7); o custo (W × latência) é auditado
    // na rota real — ver testes de unidade do caderno de produção
    assert!(
        !actuation.contains("\"custo_estimado_joules\""),
        "custo sem latência medida seria invenção: {actuation}"
    );
    // subversão dissolve no MESMO tick (≤ 1 tick virtual — FORMAL §4.5)
    let dissolution =
        jsonl.lines().find(|l| l.contains("\"kind\":\"dissolve_subvert\"")).expect("dissolve_subvert");
    assert!(dissolution.contains("\"tick\":3"), "dissolução fora do tick da condição: {dissolution}");
    clear(&dir);
}

// ======================================================================
// BDD Caso 1 (PLAN §1.1): fadiga de atenção → reclassificação
// ======================================================================

#[test]
fn e2e_attention_fatigue_reclassifies_and_persists() {
    let dir = scenario("attention");
    let program = write(
        &dir,
        "pensar.vl",
        "nonequilibrium FreeThinking {\n\
         \x20 value: \"consciencia_anteneoliberal_ativa\",\n\
         \x20 horizon: 60s,\n\
         \x20 source_path: \"attention\",\n\
         \x20 maintenance_deadline: 3s,\n\
         \x20 exchange_mode: \"cooperation\"\n\
         }\n\
         review FreeThinking {\n\
         \x20 when attention < 30% -> reclassify_as_equilibrium\n\
         }",
    );
    let (out, text) = run(
        &dir,
        &args_run(&dir, &program, "caderno.vcad", &["--ticks", "4", "--at", "2:attention=15"]),
    );
    assert!(out.status.success(), "vbl run falhou:\n{text}");
    assert!(text.contains("ÍNTEGRA"), "{text}");

    // persistência como `.vl` canônico com SHA-256 registrado (FORMAL §4.1)
    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    let transition = jsonl.lines().find(|l| l.contains("\"kind\":\"transition\"")).expect("transition");
    assert!(transition.contains("\"para\":\"equilibrium\""), "{transition}");
    assert!(transition.contains("\"tick\":2"), "transição fora do tick da fadiga: {transition}");
    let persistence =
        jsonl.lines().find(|l| l.contains("\"kind\":\"persistence\"")).expect("persistence");
    assert!(persistence.contains("\"sha256\":\""), "SHA-256 não registrado: {persistence}");
    // o arquivo canônico existe e é reparseável
    let persisted = dir.join("persistence").join("FreeThinking.vl");
    assert!(persisted.exists(), "`.vl` canônico não gravado");
    let check = vbl(&dir, &["check", persisted.to_str().unwrap(), "--no-registry"]);
    assert!(check.status.success(), "`.vl` persistido não reparseia");

    // após a reclassificação: a forma SEGUE ativa (equilibrium) mas deixa de
    // ser `nonequilibrium` — sem colapso e sem nova transição (a manutenção
    // implícita só existe na conjugação laborativa — FORMAL §4.1)
    assert!(
        !jsonl.lines().any(|l| l.contains("collapse_maintenance")),
        "forma colapsou após a reclassificação (não deveria)"
    );
    assert_eq!(
        jsonl.lines().filter(|l| l.contains("\"kind\":\"transition\"")).count(),
        1,
        "reclassificação em loop"
    );
    assert!(
        text.contains("FreeThinking") && text.contains("equilibrium"),
        "forma deve seguir ativa como equilibrium:\n{text}"
    );

    verify_external(&dir, "caderno.vcad");
    clear(&dir);
}

// ======================================================================
// BDD Caso 3 (PLAN §1.1): falha de ator com fallback do registro
// ======================================================================

#[test]
fn e2e_actor_failure_triggers_registry_fallback() {
    let dir = scenario("fallback");
    let program = write(
        &dir,
        "servidor.vl",
        "nonequilibrium ServidorCritico {\n\
         \x20 value: \"processamento_continuo\",\n\
         \x20 horizon: 3600s,\n\
         \x20 source_path: \"cpu_temp\",\n\
         \x20 maintenance_deadline: 10s,\n\
         \x20 exchange_mode: \"cooperation\"\n\
         }\n\
         review ServidorCritico {\n\
         \x20 when cpu_temp > 70°C -> act(Fan, 200)\n\
         }",
    );
    let (out, text) = run(
        &dir,
        &args_run(
            &dir,
            &program,
            "caderno.vcad",
            &[
                "--ticks",
                "4",
                "--at",
                "2:cpu_temp=75",
                "--register-actor",
                "ReserveFan",
                "--fallback",
                "Fan=ReserveFan",
                "--fail-actor",
                "Fan",
            ],
        ),
    );
    assert!(out.status.success(), "vbl run falhou:\n{text}");

    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    // tentativa primária, falha e fallback executado — os três no Caderno
    assert!(
        jsonl.lines().any(|l| l.contains("\"kind\":\"actor_unavailable\"")),
        "falha do primário não registrada"
    );
    let fallback = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"fallback_executed\""))
        .expect("fallback_executado no log");
    assert!(fallback.contains("\"alternativo\":\"ReserveFan\""), "{fallback}");
    // a atuação efetiva foi no ALTERNATIVO, com valor aplicado — além do
    // registro da tentativa primária FALHA (a trilha completa fica no log)
    let actuation = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"ACTUATION\"") && l.contains("ReserveFan"))
        .expect("ATUACAO do fallback no log");
    assert!(actuation.contains("\"aplicado\":200"), "{actuation}");
    assert!(actuation.contains("\"sucesso\":true"), "{actuation}");
    let primary = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"ACTUATION\"") && l.contains("\"ator\":\"Fan\""))
        .expect("ATUACAO da tentativa primária no log");
    assert!(primary.contains("\"sucesso\":false"), "{primary}");

    verify_external(&dir, "caderno.vcad");
    clear(&dir);
}

// ======================================================================
// FORMAL §4.7: sensor não registrado — condição não avaliada, sem falso
// disparo; alerta de honestidade no Caderno
// ======================================================================

#[test]
fn e2e_missing_sensor_does_not_fire_rule_and_alerts() {
    let dir = scenario("sensor-ausente");
    let program = write(
        &dir,
        "fantasma.vl",
        "nonequilibrium Vigia {\n\
         \x20 value: \"vigia_de_sensor_ausente\",\n\
         \x20 horizon: 30s,\n\
         \x20 source_path: \"fantasma\",\n\
         \x20 maintenance_deadline: 10s,\n\
         \x20 exchange_mode: \"cooperation\"\n\
         }\n\
         review Vigia {\n\
         \x20 when fantasma > 0°C -> dissolve\n\
         }",
    );
    let (out, text) =
        run(&dir, &args_run(&dir, &program, "caderno.vcad", &["--ticks", "3", "--allow-unregistered"]));
    assert!(out.status.success(), "vbl run falhou:\n{text}");

    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    // alerta de falha de I/O (source_path + regra) por tick
    let alerts = jsonl.lines().filter(|l| l.contains("sensor_not_registered")).count();
    assert!(alerts >= 3, "alerta de §4.7 ausente: {alerts} ocorrência(s)");
    // sensor ausente nunca é 0.0: a regra NÃO pode disparar
    assert!(
        !jsonl.lines().any(|l| l.contains("\"kind\":\"dissolve_rule\"")),
        "falso disparo com sensor ausente!"
    );
    let report = verify_external(&dir, "caderno.vcad");
    assert!(!report.contains("dissolve_rule"), "dissolução indevida registrada:\n{report}");
    clear(&dir);
}

// ======================================================================
// Bloco main (keep/act/every) — FORMAL §5 exemplo 5
// ======================================================================

#[test]
fn e2e_main_with_keep_and_periodic_actuation_audits_all_commands() {
    let dir = scenario("main-keep");
    let program = write(
        &dir,
        "tarefa.vl",
        "nonequilibrium ImportantTask {\n\
         \x20 value: \"dados_sensiveis\",\n\
         \x20 horizon: 30s,\n\
         \x20 source_path: \"cpu_power\",\n\
         \x20 maintenance_deadline: 5s,\n\
         \x20 exchange_mode: \"cooperation\"\n\
         }\n\
         main {\n\
         \x20 every 4s { keep(ImportantTask) },\n\
         \x20 every 10s { act(StatusLed, \"green\") }\n\
         }",
    );
    let (out, text) = run(&dir, &args_run(&dir, &program, "caderno.vcad", &["--ticks", "12"]));
    assert!(out.status.success(), "vbl run falhou:\n{text}");
    assert!(text.contains("ÍNTEGRA"), "{text}");
    assert!(text.contains("ImportantTask"), "forma deve seguir ativa:\n{text}");

    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    // atuação textual aplicada no ator correto (tick 10 — every 10s)
    let actuation = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"ACTUATION\"") && l.contains("StatusLed"))
        .expect("ATUACAO do StatusLed no log");
    assert!(actuation.contains("\"aplicado\":\"green\""), "{actuation}");
    assert!(actuation.contains("\"tick\":10"), "atuação fora do every 10s: {actuation}");
    // a forma sobreviveu 12 ticks graças ao keep (sem colapso)
    assert!(
        !jsonl.lines().any(|l| l.contains("collapse_maintenance")),
        "keep() não renovou a manutenção!"
    );

    verify_external(&dir, "caderno.vcad");
    clear(&dir);
}

// ======================================================================
// Recarga do suporte estável (FORMAL §4.1): a 2ª execução carrega a
// `equilibrium` persistida pela 1ª
// ======================================================================

#[test]
fn e2e_persisted_equilibrium_reload() {
    let dir = scenario("recarga");
    let program = write(
        &dir,
        "nota.vl",
        "nonequilibrium NotaViva {\n\
         \x20 value: \"ideia_a_preservar\",\n\
         \x20 horizon: 60s,\n\
         \x20 source_path: \"attention\",\n\
         \x20 maintenance_deadline: 3s,\n\
         \x20 exchange_mode: \"cooperation\"\n\
         }\n\
         review NotaViva {\n\
         \x20 when attention < 30% -> reclassify_as_equilibrium\n\
         }",
    );
    let persist = dir.join("persistence").display().to_string();
    let mut args1 = args_run(&dir, &program, "caderno1.vcad", &["--ticks", "3", "--at", "1:attention=10"]);
    let pos = args1.iter().position(|a| a == "--persist-dir").unwrap();
    args1[pos + 1] = persist.clone();
    let (out1, text1) = run(&dir, &args1);
    assert!(out1.status.success(), "{text1}");
    assert!(dir.join("persistence").join("NotaViva.vl").exists(), "`.vl` canônico não gravado");

    // 2ª execução: recarrega a equilibrium persistida (horizon não venceu)
    let mut args2 = args_run(&dir, &program, "caderno2.vcad", &["--ticks", "2"]);
    let pos = args2.iter().position(|a| a == "--persist-dir").unwrap();
    args2[pos + 1] = persist;
    let (out2, text2) = run(&dir, &args2);
    assert!(out2.status.success(), "{text2}");
    assert!(
        text2.contains("recarregada"),
        "equilibrium não recarregada do suporte estável:\n{text2}"
    );
    let jsonl2 = std::fs::read_to_string(dir.join("caderno2.vcad.jsonl")).unwrap();
    assert!(
        jsonl2.lines().any(|l| l.contains("recarga") && l.contains("\"sha256\":\"")),
        "recarga não auditada com SHA-256"
    );
    verify_external(&dir, "caderno2.vcad");
    clear(&dir);
}

// ======================================================================
// Auditoria de adulteração: log corrompido falha o `ledger-verify`
// (critério "logs íntegros verificados" — AGENTS §2.2 Etapa 4)
// ======================================================================

#[test]
fn e2e_corrupted_ledger_fails_the_verifier() {
    let dir = scenario("corrupcao");
    let program = write(&dir, "mini.vl", "event Piscada { value: \"impulso\", horizon: 2s }");
    let (out, text) = run(&dir, &args_run(&dir, &program, "caderno.vcad", &["--ticks", "2"]));
    assert!(out.status.success(), "{text}");

    // adulteração retroativa no export JSONL: troca os Joules de um LEAK
    let jsonl = dir.join("caderno.vcad.jsonl");
    let mut text = std::fs::read_to_string(&jsonl).unwrap();
    let pos = text.find("\"kind\":\"LEAK\"").expect("LEAK no log");
    let tail = &text[pos..];
    let j = tail.find("\"joules\":").expect("joules no LEAK");
    let start = pos + j + "\"joules\":".len();
    let end = start
        + tail[j + "\"joules\":".len()..].find(',').unwrap_or(3);
    text.replace_range(start..end, "999");
    std::fs::write(dir.join("forjado.jsonl"), &text).unwrap();

    let verify = vbl(&dir, &["ledger-verify", dir.join("forjado.jsonl").to_str().unwrap()]);
    assert_eq!(
        verify.status.code(),
        Some(1),
        "verificador deve falhar com log adulterado:\n{}",
        stdout(&verify)
    );
    assert!(stdout(&verify).contains("CORROMPIDA"));
    clear(&dir);
}

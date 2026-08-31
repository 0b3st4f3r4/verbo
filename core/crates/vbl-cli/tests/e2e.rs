//! E2E da Etapa 4 (PLAN §4.2): o interpretador integrado (CLI `vbl` + FXP +
//! Caderno de produção) submetido aos cenários comportamentais da Etapa 1.
//!
//! Cada cenário roda o binário de verdade, exporta o log do Caderno
//! (binário + JSONL), verifica a integridade da cadeia SHA-256 com o
//! verificador externo (`vbl caderno-verify`) e audita as atuações
//! registradas (valor solicitado/aplicado, latência, custo) — critérios de
//! "Pronto" da Etapa 4 (AGENTS §2.2).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Diretório por cenário (isolado por PID; limpeza best-effort no fim).
fn cenario(nome: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vbl-e2e-{nome}-{}", std::process::id()));
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

fn escrever(dir: &Path, nome: &str, conteudo: &str) -> String {
    let caminho = dir.join(nome);
    std::fs::write(&caminho, conteudo).unwrap();
    caminho.display().to_string()
}

/// Linha de comando completa: programa + extras + persistência + Caderno.
fn args_run(dir: &Path, programa: &str, caderno: &str, extras: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["run", programa].iter().map(|s| s.to_string()).collect();
    for e in extras {
        args.push(e.to_string());
    }
    args.push("--persist-dir".into());
    args.push(dir.join("persistencia").display().to_string());
    args.push("--caderno".into());
    args.push(dir.join(caderno).display().to_string());
    args
}

fn rodar(dir: &Path, args: &[String]) -> (Output, String) {
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = vbl(dir, &refs);
    let texto = stdout(&out);
    (out, texto)
}

/// Verificação externa do Caderno do cenário (deve sair ÍNTEGRA).
fn verificar_externo(dir: &Path, caderno: &str) -> String {
    let verify = vbl(dir, &["caderno-verify", dir.join(caderno).to_str().unwrap()]);
    let relatorio = stdout(&verify);
    assert!(verify.status.success(), "caderno-verify falhou:\n{relatorio}");
    assert!(relatorio.contains("ÍNTEGRA"), "cadeia corrompida:\n{relatorio}");
    relatorio
}

fn limpar(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

// ======================================================================
// BDD Caso 2 (PLAN §1.1): subversão poética por sobrecarga térmica
// ======================================================================

#[test]
fn e2e_subversao_termica_atua_no_ator_e_audita() {
    let dir = cenario("subversao");
    let programa = escrever(
        &dir,
        "trading.vl",
        "nonequilibrium TradingEspeculativo {\n\
         \x20 value: \"lucro_arbitragem_alta_frequencia\",\n\
         \x20 horizon: 7s,\n\
         \x20 source_path: \"cpu_temp\",\n\
         \x20 maintenance_deadline: 2s,\n\
         \x20 exchange_mode: \"extraction\"\n\
         }\n\
         review TradingEspeculativo {\n\
         \x20 when cpu_temp > 85°C -> subvert,\n\
         \x20                         act(CpuPowerCap, 50)\n\
         }",
    );
    let (out, texto) = rodar(
        &dir,
        &args_run(&dir, &programa, "caderno.vcad", &["--ticks", "5", "--at", "3:cpu_temp=86.5"]),
    );
    assert!(out.status.success(), "vbl run falhou:\n{texto}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(texto.contains("ÍNTEGRA"), "cadeia não íntegra:\n{texto}");
    assert!(texto.contains("atuações 1/1 ok"), "atuação não confirmada:\n{texto}");

    let relatorio = verificar_externo(&dir, "caderno.vcad");
    assert!(relatorio.contains("ATUACAO: 1"), "atuação não registrada:\n{relatorio}");

    // atuação auditada: solicitado = aplicado = 50, no tick da condição
    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    let atuacao = jsonl.lines().find(|l| l.contains("\"kind\":\"ATUACAO\"")).expect("ATUACAO no log");
    for esperado in [
        "\"ator\":\"CpuPowerCap\"",
        "\"valor\":50",
        "\"aplicado\":50",
        "\"sucesso\":true",
        "\"tick\":3",
        "\"t\":3",
    ] {
        assert!(atuacao.contains(esperado), "ATUACAO sem {esperado}: {atuacao}");
    }
    // no modo SIMULADO a fronteira é em processo: sem latência física ⇒ sem
    // custo estimado (honestidade §4.7); o custo (W × latência) é auditado
    // na rota real — ver testes de unidade do caderno de produção
    assert!(
        !atuacao.contains("\"custo_estimado_joules\""),
        "custo sem latência medida seria invenção: {atuacao}"
    );
    // subversão dissolve no MESMO tick (≤ 1 tick virtual — FORMAL §4.5)
    let dissolucao =
        jsonl.lines().find(|l| l.contains("\"kind\":\"dissolve_subvert\"")).expect("dissolve_subvert");
    assert!(dissolucao.contains("\"tick\":3"), "dissolução fora do tick da condição: {dissolucao}");
    limpar(&dir);
}

// ======================================================================
// BDD Caso 1 (PLAN §1.1): fadiga de atenção → reclassificação
// ======================================================================

#[test]
fn e2e_fadiga_de_atencao_reclassifica_e_persiste() {
    let dir = cenario("atencao");
    let programa = escrever(
        &dir,
        "pensar.vl",
        "nonequilibrium PensarLivre {\n\
         \x20 value: \"consciencia_anteneoliberal_ativa\",\n\
         \x20 horizon: 60s,\n\
         \x20 source_path: \"attention\",\n\
         \x20 maintenance_deadline: 3s,\n\
         \x20 exchange_mode: \"cooperation\"\n\
         }\n\
         review PensarLivre {\n\
         \x20 when attention < 30% -> reclassify_as_equilibrium\n\
         }",
    );
    let (out, texto) = rodar(
        &dir,
        &args_run(&dir, &programa, "caderno.vcad", &["--ticks", "4", "--at", "2:attention=15"]),
    );
    assert!(out.status.success(), "vbl run falhou:\n{texto}");
    assert!(texto.contains("ÍNTEGRA"), "{texto}");

    // persistência como `.vl` canônico com SHA-256 registrado (FORMAL §4.1)
    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    let transicao = jsonl.lines().find(|l| l.contains("\"kind\":\"transicao\"")).expect("transicao");
    assert!(transicao.contains("\"para\":\"equilibrium\""), "{transicao}");
    assert!(transicao.contains("\"tick\":2"), "transição fora do tick da fadiga: {transicao}");
    let persistencia =
        jsonl.lines().find(|l| l.contains("\"kind\":\"persistencia\"")).expect("persistencia");
    assert!(persistencia.contains("\"sha256\":\""), "SHA-256 não registrado: {persistencia}");
    // o arquivo canônico existe e é reparseável
    let persistido = dir.join("persistencia").join("PensarLivre.vl");
    assert!(persistido.exists(), "`.vl` canônico não gravado");
    let check = vbl(&dir, &["check", persistido.to_str().unwrap(), "--sem-registro"]);
    assert!(check.status.success(), "`.vl` persistido não reparseia");

    // após a reclassificação: a forma SEGUE ativa (equilibrium) mas deixa de
    // ser `nonequilibrium` — sem colapso e sem nova transição (a manutenção
    // implícita só existe na conjugação laborativa — FORMAL §4.1)
    assert!(
        !jsonl.lines().any(|l| l.contains("collapse_maintenance")),
        "forma colapsou após a reclassificação (não deveria)"
    );
    assert_eq!(
        jsonl.lines().filter(|l| l.contains("\"kind\":\"transicao\"")).count(),
        1,
        "reclassificação em loop"
    );
    assert!(
        texto.contains("PensarLivre") && texto.contains("equilibrium"),
        "forma deve seguir ativa como equilibrium:\n{texto}"
    );

    verificar_externo(&dir, "caderno.vcad");
    limpar(&dir);
}

// ======================================================================
// BDD Caso 3 (PLAN §1.1): falha de ator com fallback do registro
// ======================================================================

#[test]
fn e2e_falha_de_ator_aciona_fallback_do_registro() {
    let dir = cenario("fallback");
    let programa = escrever(
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
         \x20 when cpu_temp > 70°C -> act(Ventoinha, 200)\n\
         }",
    );
    let (out, texto) = rodar(
        &dir,
        &args_run(
            &dir,
            &programa,
            "caderno.vcad",
            &[
                "--ticks",
                "4",
                "--at",
                "2:cpu_temp=75",
                "--registrar-ator",
                "VentoinhaReserva",
                "--fallback",
                "Ventoinha=VentoinhaReserva",
                "--falhar-ator",
                "Ventoinha",
            ],
        ),
    );
    assert!(out.status.success(), "vbl run falhou:\n{texto}");

    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    // tentativa primária, falha e fallback executado — os três no Caderno
    assert!(
        jsonl.lines().any(|l| l.contains("\"kind\":\"ator_indisponivel\"")),
        "falha do primário não registrada"
    );
    let fallback = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"fallback_executado\""))
        .expect("fallback_executado no log");
    assert!(fallback.contains("\"alternativo\":\"VentoinhaReserva\""), "{fallback}");
    // a atuação efetiva foi no ALTERNATIVO, com valor aplicado — além do
    // registro da tentativa primária FALHA (a trilha completa fica no log)
    let atuacao = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"ATUACAO\"") && l.contains("VentoinhaReserva"))
        .expect("ATUACAO do fallback no log");
    assert!(atuacao.contains("\"aplicado\":200"), "{atuacao}");
    assert!(atuacao.contains("\"sucesso\":true"), "{atuacao}");
    let primaria = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"ATUACAO\"") && l.contains("\"ator\":\"Ventoinha\""))
        .expect("ATUACAO da tentativa primária no log");
    assert!(primaria.contains("\"sucesso\":false"), "{primaria}");

    verificar_externo(&dir, "caderno.vcad");
    limpar(&dir);
}

// ======================================================================
// FORMAL §4.7: sensor não registrado — condição não avaliada, sem falso
// disparo; alerta de honestidade no Caderno
// ======================================================================

#[test]
fn e2e_sensor_ausente_nao_dispara_regra_e_alerta() {
    let dir = cenario("sensor-ausente");
    let programa = escrever(
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
    let (out, texto) =
        rodar(&dir, &args_run(&dir, &programa, "caderno.vcad", &["--ticks", "3", "--permitir-sem-registro"]));
    assert!(out.status.success(), "vbl run falhou:\n{texto}");

    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    // alerta de falha de I/O (source_path + regra) por tick
    let alertas = jsonl.lines().filter(|l| l.contains("sensor_nao_registrado")).count();
    assert!(alertas >= 3, "alerta de §4.7 ausente: {alertas} ocorrência(s)");
    // sensor ausente nunca é 0.0: a regra NÃO pode disparar
    assert!(
        !jsonl.lines().any(|l| l.contains("\"kind\":\"dissolve_rule\"")),
        "falso disparo com sensor ausente!"
    );
    let relatorio = verificar_externo(&dir, "caderno.vcad");
    assert!(!relatorio.contains("dissolve_rule"), "dissolução indevida registrada:\n{relatorio}");
    limpar(&dir);
}

// ======================================================================
// Bloco main (keep/act/every) — FORMAL §5 exemplo 5
// ======================================================================

#[test]
fn e2e_main_com_keep_e_atuacao_periodica_audita_todos_os_comandos() {
    let dir = cenario("main-keep");
    let programa = escrever(
        &dir,
        "tarefa.vl",
        "nonequilibrium TarefaImportante {\n\
         \x20 value: \"dados_sensiveis\",\n\
         \x20 horizon: 30s,\n\
         \x20 source_path: \"cpu_power\",\n\
         \x20 maintenance_deadline: 5s,\n\
         \x20 exchange_mode: \"cooperation\"\n\
         }\n\
         main {\n\
         \x20 every 4s { keep(TarefaImportante) },\n\
         \x20 every 10s { act(LedIndicador, \"verde\") }\n\
         }",
    );
    let (out, texto) = rodar(&dir, &args_run(&dir, &programa, "caderno.vcad", &["--ticks", "12"]));
    assert!(out.status.success(), "vbl run falhou:\n{texto}");
    assert!(texto.contains("ÍNTEGRA"), "{texto}");
    assert!(texto.contains("TarefaImportante"), "forma deve seguir ativa:\n{texto}");

    let jsonl = std::fs::read_to_string(dir.join("caderno.vcad.jsonl")).unwrap();
    // atuação textual aplicada no ator correto (tick 10 — every 10s)
    let atuacao = jsonl
        .lines()
        .find(|l| l.contains("\"kind\":\"ATUACAO\"") && l.contains("LedIndicador"))
        .expect("ATUACAO do LedIndicador no log");
    assert!(atuacao.contains("\"aplicado\":\"verde\""), "{atuacao}");
    assert!(atuacao.contains("\"tick\":10"), "atuação fora do every 10s: {atuacao}");
    // a forma sobreviveu 12 ticks graças ao keep (sem colapso)
    assert!(
        !jsonl.lines().any(|l| l.contains("collapse_maintenance")),
        "keep() não renovou a manutenção!"
    );

    verificar_externo(&dir, "caderno.vcad");
    limpar(&dir);
}

// ======================================================================
// Recarga do suporte estável (FORMAL §4.1): a 2ª execução carrega a
// `equilibrium` persistida pela 1ª
// ======================================================================

#[test]
fn e2e_recarga_de_equilibrium_persistida() {
    let dir = cenario("recarga");
    let programa = escrever(
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
    let persist = dir.join("persistencia").display().to_string();
    let mut args1 = args_run(&dir, &programa, "caderno1.vcad", &["--ticks", "3", "--at", "1:attention=10"]);
    let pos = args1.iter().position(|a| a == "--persist-dir").unwrap();
    args1[pos + 1] = persist.clone();
    let (out1, texto1) = rodar(&dir, &args1);
    assert!(out1.status.success(), "{texto1}");
    assert!(dir.join("persistencia").join("NotaViva.vl").exists(), "`.vl` canônico não gravado");

    // 2ª execução: recarrega a equilibrium persistida (horizon não venceu)
    let mut args2 = args_run(&dir, &programa, "caderno2.vcad", &["--ticks", "2"]);
    let pos = args2.iter().position(|a| a == "--persist-dir").unwrap();
    args2[pos + 1] = persist;
    let (out2, texto2) = rodar(&dir, &args2);
    assert!(out2.status.success(), "{texto2}");
    assert!(
        texto2.contains("recarregada"),
        "equilibrium não recarregada do suporte estável:\n{texto2}"
    );
    let jsonl2 = std::fs::read_to_string(dir.join("caderno2.vcad.jsonl")).unwrap();
    assert!(
        jsonl2.lines().any(|l| l.contains("recarga") && l.contains("\"sha256\":\"")),
        "recarga não auditada com SHA-256"
    );
    verificar_externo(&dir, "caderno2.vcad");
    limpar(&dir);
}

// ======================================================================
// Auditoria de adulteração: log corrompido falha o `caderno-verify`
// (critério "logs íntegros verificados" — AGENTS §2.2 Etapa 4)
// ======================================================================

#[test]
fn e2e_caderno_corrompido_falha_o_verificador() {
    let dir = cenario("corrupcao");
    let programa = escrever(&dir, "mini.vl", "event Piscada { value: \"impulso\", horizon: 2s }");
    let (out, texto) = rodar(&dir, &args_run(&dir, &programa, "caderno.vcad", &["--ticks", "2"]));
    assert!(out.status.success(), "{texto}");

    // adulteração retroativa no export JSONL: troca os Joules de um VAZAMENTO
    let jsonl = dir.join("caderno.vcad.jsonl");
    let mut texto = std::fs::read_to_string(&jsonl).unwrap();
    let pos = texto.find("\"kind\":\"VAZAMENTO\"").expect("VAZAMENTO no log");
    let cauda = &texto[pos..];
    let j = cauda.find("\"joules\":").expect("joules no VAZAMENTO");
    let inicio = pos + j + "\"joules\":".len();
    let fim = inicio
        + cauda[j + "\"joules\":".len()..].find(',').unwrap_or(3);
    texto.replace_range(inicio..fim, "999");
    std::fs::write(dir.join("forjado.jsonl"), &texto).unwrap();

    let verify = vbl(&dir, &["caderno-verify", dir.join("forjado.jsonl").to_str().unwrap()]);
    assert_eq!(
        verify.status.code(),
        Some(1),
        "verificador deve falhar com log adulterado:\n{}",
        stdout(&verify)
    );
    assert!(stdout(&verify).contains("CORROMPIDA"));
    limpar(&dir);
}

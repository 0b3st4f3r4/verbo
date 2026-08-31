//! Testes de transição — reancoragem Rust da suíte da Etapa 1
//! (`tests/unit/test_finitude.py`, `test_tick.py`, `test_atores.py`,
//! `test_falha_sensores.py` e as cláusulas de runtime de
//! `test_clausulas_erro.py`).
//!
//! Os programas são construídos como TEXTO `.vl` e passam pelo parser real
//! da Etapa 2 (`vbl-lang`) — o caminho de produção completo:
//! texto → AST → loader → engine.

use vbl_runtime::caderno::kinds;
use vbl_runtime::forma::{ORCAMENTO_RETENCAO, VALOR_POETICO_CANONICO};
use vbl_runtime::json::Json;
use vbl_runtime::scheduler::Prazo;
use vbl_runtime::{
    carregar, ActorLimits, Engine, FxpSimulator, MainInterpreter,
};
use vbl_lang::parse;

type Eng = Engine<FxpSimulator>;

/// Banco de teste: engine + FXP simulado + interpretador do main.
struct Banco {
    engine: Eng,
    interp: Option<MainInterpreter>,
}

fn sim_padrao() -> FxpSimulator {
    FxpSimulator::novo()
}

/// Parse + carga no engine. `persist` = diretório de persistência.
fn montar_com(fxp: FxpSimulator, fonte: &str, persist: &std::path::Path) -> Banco {
    let (programa, diags) = parse(fonte);
    assert!(!diags.has_errors(), "programa de teste inválido:\n{diags}\n{fonte}");
    let mut engine = Engine::novo(fxp, 1.0, persist);
    let interp = carregar(&mut engine, &programa);
    let interp = if programa.main.is_some() { Some(interp) } else { None };
    Banco { engine, interp }
}

fn montar_em(fxp: FxpSimulator, fonte: &str, dir: &std::path::Path) -> Banco {
    let mut b = montar_com(fxp, fonte, dir);
    // recarga de equilibria persistidos (FORMAL §4.1 — na inicialização)
    if std::fs::read_dir(dir).is_ok() {
        vbl_runtime::persist::recarregar_equilibrium(&mut b.engine);
    }
    b
}

fn montar(fxp: FxpSimulator, fonte: &str) -> Banco {
    let dir = std::env::temp_dir().join(format!(
        "vbl-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    montar_em(fxp, fonte, &dir)
}

impl Banco {
    fn tick(&mut self) {
        if let Some(interp) = &mut self.interp {
            interp.run_due(&mut self.engine);
        }
        self.engine.tick();
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            self.tick();
        }
    }

    fn set_sensor(&mut self, nome: &str, valor: f64) {
        self.engine.fxp.set_sensor(nome, valor);
    }

    fn tem(&self, kind: &str) -> bool {
        self.engine.caderno.tem(kind)
    }

    fn tem_com(&self, kind: &str, filtro: &[(&str, Json)]) -> bool {
        self.engine.caderno.tem_com(kind, filtro)
    }

    fn contar(&self, kind: &str) -> usize {
        self.engine.caderno.kinds().iter().filter(|k| **k == kind).count()
    }

    fn forma_viva(&self, nome: &str) -> bool {
        self.engine.forma(nome).is_some()
    }
}

impl Drop for Banco {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.engine.persistence_dir());
    }
}

// ======================================================================
// Finitude (test_finitude.py) — FORMAL §4.1
// ======================================================================
#[test]
fn event_expira_no_horizon_com_fim_tipificado() {
    let mut b = montar(sim_padrao(), "event Piscada { value: \"impulso_curto\", horizon: 3s }");
    b.tick(); // t=1
    b.tick(); // t=2 — ainda ativa
    assert!(b.forma_viva("Piscada"));
    b.tick(); // t=3 — horizon esgota (>=)
    assert!(!b.forma_viva("Piscada"));
    assert!(b.tem(kinds::DISSOLVE_HORIZON));
}

#[test]
fn horizon_e_absoluto_reclassificacao_nao_renova() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Pensar { value: \"ideia\", horizon: 5s, source_path: \"attention\", maintenance_deadline: 3s }\n\
         review Pensar { when attention < 30% -> reclassify_as_equilibrium }",
    );
    b.set_sensor("attention", 15.0);
    b.tick(); // t=1: reclassifica (persistida)
    assert!(b.tem_com(
        kinds::TRANSICAO,
        &[("forma", Json::str("Pensar")), ("para", Json::str("equilibrium"))]
    ));
    assert_eq!(b.engine.forma("Pensar").unwrap().creation_time, 0.0); // não renovado
    assert!(b.tem(kinds::PERSISTENCIA)); // gravação `.vl` canônico
    b.set_sensor("attention", 90.0); // regra para de disparar
    b.ticks(3); // t=2,3,4
    assert!(b.forma_viva("Pensar"));
    b.tick(); // t=5: horizon original esgota
    assert!(!b.forma_viva("Pensar"));
    assert!(b.tem(kinds::DISSOLVE_HORIZON));
}

#[test]
fn equilibrium_tambem_expira_por_horizon() {
    let mut b = montar(sim_padrao(), "equilibrium Registro { value: \"doc\", horizon: 2s }");
    b.tick();
    assert!(b.forma_viva("Registro")); // t=1: 1>=2? não
    b.tick(); // t=2: limite exato expira
    assert!(!b.forma_viva("Registro"));
    assert!(b.tem(kinds::DISSOLVE_HORIZON));
}

#[test]
fn nonequilibrium_com_regra_ativa_tem_manutencao_implicita() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Vigilia { value: \"trabalho\", horizon: 30s, source_path: \"attention\", maintenance_deadline: 2s }\n\
         review Vigilia { when attention < 5% -> dissolve }",
    );
    b.ticks(6); // attention = 100 → regra nunca dispara
    assert!(b.forma_viva("Vigilia"));
    assert!(!b.tem(kinds::COLLAPSE_MAINTENANCE));
}

#[test]
fn nonequilibrium_sem_manutencao_colapsa_no_primeiro_vencimento() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Solo { value: \"sem_vigilia\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 2s }",
    );
    b.tick(); // t=1: 1 > 2? não
    b.tick(); // t=2: 2 > 2? não (limite estrito)
    assert!(b.forma_viva("Solo"));
    b.tick(); // t=3: 3 > 2? sim — colapso
    assert!(!b.forma_viva("Solo"));
    assert!(b.tem_com(kinds::COLLAPSE_MAINTENANCE, &[("forma", Json::str("Solo"))]));
}

#[test]
fn keep_manual_renova_o_prazo() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Solo { value: \"mantido\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 2s }\n\
         main { every 1s { keep(Solo) } }",
    );
    b.ticks(6);
    assert!(b.forma_viva("Solo"));
    assert!(!b.tem(kinds::COLLAPSE_MAINTENANCE));
}

#[test]
fn forma_termina_uma_unicamente_por_tick() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Ciclo { value: \"lucro\", horizon: 3s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review Ciclo { when cpu_temp > 85°C -> dissolve }",
    );
    b.set_sensor("cpu_temp", 90.0);
    b.tick(); // t=3 não chegou; regra dispara primeiro
    assert!(!b.forma_viva("Ciclo"));
    assert_eq!(b.contar(kinds::DISSOLVE_RULE), 1);
    assert!(!b.tem(kinds::DISSOLVE_HORIZON));
}

#[test]
fn contadores_de_retencao_dentro_dos_orcamentos() {
    let mut b = montar(
        sim_padrao(),
        "event Ev { value: \"curto\", horizon: 3s }\n\
         equilibrium Eq { value: \"doc\", horizon: 30s, cost_bytes: 128 }\n\
         nonequilibrium Neq { value: \"trabalho\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 2s }",
    );
    let (event_o, eq_o, neq_o) = ORCAMENTO_RETENCAO;
    assert!(b.engine.retencao.por_forma["Ev"] <= event_o);
    assert!(b.engine.retencao.por_forma["Eq"] <= eq_o);
    assert!(b.engine.retencao.por_forma["Neq"] <= neq_o);
    assert!(b.engine.retencao.labor["Neq"] > 0);
    b.engine.dissolve_form("Neq", kinds::DISSOLVE_RULE);
    assert!(!b.engine.retencao.por_forma.contains_key("Neq"));
    assert!(!b.engine.retencao.labor.contains_key("Neq")); // 0 bytes retidos
}

// ======================================================================
// Ordem e precedência no tick (test_tick.py) — FORMAL §4.2/§4.5
// ======================================================================
#[test]
fn regras_sao_avaliadas_na_ordem_declarada() {
    let mut b = montar(
        sim_padrao(),
        "event Dupla { value: \"v\", horizon: 30s }\n\
         review Dupla { when cpu_temp > 10°C -> act(Ventoinha, 100),\n\
                        when cpu_temp > 10°C -> act(Ventoinha, 150) }",
    );
    b.set_sensor("cpu_temp", 40.0);
    b.tick();
    let entregas: Vec<f64> = b
        .engine
        .fxp
        .entregues
        .iter()
        .filter(|m| m.ator == "Ventoinha")
        .filter_map(|m| m.valor.as_num())
        .collect();
    assert_eq!(entregas, vec![100.0, 150.0]); // ordem declarada preservada
}

#[test]
fn review_short_circuit_apos_dissolucao() {
    let mut b = montar(
        sim_padrao(),
        "event Curto { value: \"v\", horizon: 30s }\n\
         review Curto { when cpu_temp > 10°C -> act(Ventoinha, 100), dissolve,\n\
                        when cpu_temp > 10°C -> act(LedIndicador, \"vermelho\") }",
    );
    b.set_sensor("cpu_temp", 40.0);
    b.tick();
    assert!(b.tem_com(
        kinds::REVIEW_SHORT_CIRCUIT,
        &[("forma", Json::str("Curto")), ("regras_restantes", Json::num(1.0))]
    ));
    assert!(b.tem(kinds::DISSOLVE_RULE));
    assert!(b.engine.fxp.entregues.iter().any(|m| m.ator == "Ventoinha"));
    assert!(!b.engine.fxp.entregues.iter().any(|m| m.ator == "LedIndicador"));
}

#[test]
fn subvert_nao_cancela_act_da_mesma_regra() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Trading { value: \"lucro\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review Trading { when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50) }",
    );
    b.set_sensor("cpu_temp", 86.5);
    b.tick();
    assert!(b.tem_com(kinds::DISSOLVE_SUBVERT, &[("forma", Json::str("Trading"))]));
    assert!(b.tem_com(
        kinds::SUBVERT_APLICADO,
        &[("novo_valor", Json::str(VALOR_POETICO_CANONICO))]
    ));
    assert!(b
        .engine
        .fxp
        .entregues
        .iter()
        .any(|m| m.ator == "CpuPowerCap" && m.valor == vbl_runtime::Value::Num(50.0)));
    assert!(!b.forma_viva("Trading")); // dissolvida no mesmo tick
    assert_eq!(b.engine.clock, 1); // ≤ 1 tick virtual
}

#[test]
fn atuacao_despachada_nao_e_revogada_pelo_subvert() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium T { value: \"v\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review T { when cpu_temp > 85°C -> subvert, act(LedIndicador, \"verde\") }",
    );
    b.set_sensor("cpu_temp", 90.0);
    b.tick();
    assert_eq!(
        b.engine.fxp.ator_atual("LedIndicador"),
        Some(&vbl_runtime::Value::Str("verde".into()))
    );
}

#[test]
fn notify_shutdown_nao_dissolve_nem_interrompe() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium T { value: \"v\", horizon: 30s, source_path: \"attention\", maintenance_deadline: 10s }\n\
         review T { when attention < 20% -> notify_shutdown, act(LedIndicador, \"apagado\") }",
    );
    b.set_sensor("attention", 10.0);
    b.tick();
    assert!(b.forma_viva("T")); // forma permanece ativa
    assert!(!b.tem(kinds::DISSOLVE_RULE));
    assert_eq!(
        b.engine.fxp.ator_atual("LedIndicador"),
        Some(&vbl_runtime::Value::Str("apagado".into()))
    );
}

#[test]
fn partilha_igual_da_potencia_global() {
    let mut b = montar(
        sim_padrao(),
        "event A { value: \"a\", horizon: 30s }\nevent B { value: \"b\", horizon: 30s }",
    );
    b.engine.fxp.set_sensor("cpu_power", 100.0);
    b.tick();
    let vazamentos = b.engine.caderno.buscar("VAZAMENTO", &[]);
    let por_forma: std::collections::BTreeMap<String, f64> = vazamentos
        .iter()
        .filter_map(|e| match &e.extra {
            Json::Obj(c) => Some((
                match c.get("forma") {
                    Some(Json::Str(s)) => s.clone(),
                    _ => String::new(),
                },
                match c.get("watts") {
                    Some(Json::Num(n)) => *n,
                    _ => 0.0,
                },
            )),
            _ => None,
        })
        .collect();
    assert!((por_forma["A"] - 50.0).abs() < 1e-9);
    assert!((por_forma["B"] - 50.0).abs() < 1e-9);
    assert!((por_forma["A"] + por_forma["B"] - 100.0).abs() < 1e-9);
}

#[test]
fn cadeia_sha256_detecta_adulteracao() {
    let mut b = montar(sim_padrao(), "event X { value: \"v\", horizon: 3s }");
    b.tick();
    assert!(b.engine.caderno.verify_chain());
    // adulteração retroativa quebra a cadeia
    b.engine.caderno.eventos[0].msg = "forjado".into();
    assert!(!b.engine.caderno.verify_chain());
}

#[test]
fn exportacao_jsonl_reproduz_a_cadeia() {
    let mut b = montar(sim_padrao(), "event X { value: \"v\", horizon: 3s }");
    b.tick();
    std::fs::create_dir_all(b.engine.persistence_dir()).unwrap();
    let caminho = b.engine.persistence_dir().join("caderno.jsonl");
    let n = b.engine.caderno.export_jsonl(&caminho).unwrap();
    assert!(n > 0);
    let texto = std::fs::read_to_string(&caminho).unwrap();
    let linhas: Vec<&str> = texto.lines().collect();
    assert_eq!(linhas.len(), n);
    // cada linha traz seq/kind/msg/hash — auditoria externa possível
    for (i, linha) in linhas.iter().enumerate() {
        assert!(linha.contains(&format!("\"seq\":{i}")), "linha {i}: {linha}");
        assert!(linha.contains("\"hash\":\""), "linha {i} sem hash");
    }
    // kinds na ordem
    let primeiro_kind = linhas[0].split("\"kind\":\"").nth(1).unwrap().split('"').next().unwrap();
    assert_eq!(primeiro_kind, "INFO"); // "Forma X conjugada..."
}

// ======================================================================
// Atores (test_atores.py) — FORMAL §4.3
// ======================================================================
#[test]
fn act_e_serializado_e_entregue_ao_ator_correto() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Servidor { value: \"critico\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review Servidor { when cpu_temp > 70°C -> act(Ventoinha, 200) }",
    );
    b.set_sensor("cpu_temp", 75.0);
    b.tick();
    // mensagem serializada no outbox do FXP
    let msg = b
        .engine
        .fxp
        .outbox
        .iter()
        .find(|m| m.op == "act" && m.ator == "Ventoinha" && m.valor == vbl_runtime::Value::Num(200.0))
        .expect("mensagem FXP `act` não serializada");
    assert_eq!(msg.tick, b.engine.clock); // tick de despacho registrado
    assert!(b
        .engine
        .fxp
        .entregues
        .iter()
        .any(|m| m.ator == "Ventoinha" && m.valor == vbl_runtime::Value::Num(200.0)));
    assert_eq!(b.engine.fxp.ator_atual("Ventoinha"), Some(&vbl_runtime::Value::Num(200.0)));
    assert!(b.tem_com(
        "ATUACAO",
        &[("ator", Json::str("Ventoinha")), ("valor", Json::num(200.0)), ("sucesso", Json::boolean(true))]
    ));
}

#[test]
fn ator_inexistente_rejeitado_com_registro() {
    let mut b = montar(
        sim_padrao(),
        "event Tarefa { value: \"x\", horizon: 10s }\n\
         review Tarefa { when cpu_temp > 10°C -> act(AtorFantasma, 10) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert!(b.tem_com(kinds::ATOR_INEXISTENTE, &[("ator", Json::str("AtorFantasma"))]));
    assert!(b.tem_com("ATUACAO", &[("ator", Json::str("AtorFantasma")), ("sucesso", Json::boolean(false))]));
    assert!(!b.engine.fxp.entregues.iter().any(|m| m.ator == "AtorFantasma"));
}

#[test]
fn valor_abaixo_do_minimo_rejeitado_sem_envio() {
    let mut b = montar(
        sim_padrao(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(CpuPowerCap, 5) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    let eventos = b.engine.caderno.buscar(
        kinds::ACTOR_REJECTED_VALUE,
        &[("ator", Json::str("CpuPowerCap"))],
    );
    assert_eq!(eventos.len(), 1);
    match &eventos[0].extra {
        Json::Obj(c) => {
            assert_eq!(c.get("limite"), Some(&Json::str("min")));
            assert_eq!(c.get("limite_valor"), Some(&Json::num(10.0)));
        }
        _ => panic!("extra ausente"),
    }
    assert!(!b.engine.fxp.entregues.iter().any(|m| m.ator == "CpuPowerCap"));
}

#[test]
fn valor_acima_do_safety_limit_rejeitado() {
    let mut b = montar(
        sim_padrao(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(Ventoinha, 250) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    let eventos = b.engine.caderno.buscar(
        kinds::ACTOR_REJECTED_VALUE,
        &[("ator", Json::str("Ventoinha"))],
    );
    assert_eq!(eventos.len(), 1);
    match &eventos[0].extra {
        Json::Obj(c) => {
            assert_eq!(c.get("limite"), Some(&Json::str("safety_limit")));
            assert_eq!(c.get("limite_valor"), Some(&Json::num(200.0)));
        }
        _ => panic!("extra ausente"),
    }
    assert!(!b.engine.fxp.entregues.iter().any(|m| m.ator == "Ventoinha"));
}

#[test]
fn limites_sao_inclusivos() {
    let mut b = montar(
        sim_padrao(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(Ventoinha, 200), act(CpuPowerCap, 10) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert_eq!(b.engine.fxp.ator_atual("Ventoinha"), Some(&vbl_runtime::Value::Num(200.0)));
    assert_eq!(b.engine.fxp.ator_atual("CpuPowerCap"), Some(&vbl_runtime::Value::Num(10.0)));
    assert!(!b.tem(kinds::ACTOR_REJECTED_VALUE));
}

#[test]
fn ator_fora_do_maximo_rejeitado_sem_envio() {
    let mut b = montar(
        sim_padrao(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(Ventoinha, 256) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert!(b.tem(kinds::ACTOR_REJECTED_VALUE));
    assert!(!b.engine.fxp.entregues.iter().any(|m| m.ator == "Ventoinha"));
}

#[test]
fn act_com_valor_textual_sem_limites_numericos() {
    let mut b = montar(
        sim_padrao(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(LedIndicador, \"verde\") }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert_eq!(
        b.engine.fxp.ator_atual("LedIndicador"),
        Some(&vbl_runtime::Value::Str("verde".into()))
    );
}

#[test]
fn fallback_do_registro_e_acionado_quando_primario_falha() {
    let mut fxp = FxpSimulator::novo();
    fxp.registrar_ator("VentoinhaReserva", ActorLimits { min: Some(0.0), max: Some(255.0), safety_limit: Some(200.0) });
    fxp.definir_fallback("Ventoinha", &["VentoinhaReserva"]);
    fxp.falhar_ator("Ventoinha");
    let mut b = montar(
        fxp,
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 70°C -> act(Ventoinha, 200) }",
    );
    b.set_sensor("cpu_temp", 75.0);
    b.tick();
    // tentativa primária registrada, falha registrada, fallback executado
    assert!(b.tem_com("ATUACAO", &[("ator", Json::str("Ventoinha")), ("sucesso", Json::boolean(false))]));
    assert!(b.tem_com(kinds::ATOR_INDISPONIVEL, &[("ator", Json::str("Ventoinha"))]));
    assert!(b.tem_com(
        kinds::FALLBACK_EXECUTADO,
        &[("primario", Json::str("Ventoinha")), ("alternativo", Json::str("VentoinhaReserva"))]
    ));
    assert_eq!(
        b.engine.fxp.ator_atual("VentoinhaReserva"),
        Some(&vbl_runtime::Value::Num(200.0))
    );
    assert!(b
        .engine
        .fxp
        .entregues
        .iter()
        .any(|m| m.ator == "VentoinhaReserva" && m.fallback_de.as_deref() == Some("Ventoinha")));
}

// ======================================================================
// Falha de sensores (test_falha_sensores.py) — FORMAL §4.7
// ======================================================================
const SENTINELA: &str = "\
nonequilibrium Sentinela { value: \"vigia\", horizon: 30s, source_path: \"attention\", maintenance_deadline: 3s }\n\
review Sentinela { when attention < 30% -> reclassify_as_equilibrium }";

#[test]
fn leitura_zero_e_valida_e_dispara_regras() {
    let mut b = montar(sim_padrao(), SENTINELA);
    b.set_sensor("attention", 0.0);
    b.tick();
    assert!(b.tem_com(
        kinds::TRANSICAO,
        &[("forma", Json::str("Sentinela")), ("para", Json::str("equilibrium"))]
    ));
    // zero NÃO é falha de I/O: nenhum alerta de sensor
    assert!(!b.tem_com("ALERTA", &[("motivo", Json::str("sensor_nao_registrado"))]));
    assert!(!b.tem_com("ALERTA", &[("motivo", Json::str("sensor_inacessivel"))]));
}

#[test]
fn sensor_ausente_nao_avalia_condicao_nem_dispara() {
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium Fantasma { value: \"obs\", horizon: 30s, source_path: \"sensor_inexistente\", maintenance_deadline: 3s }\n\
         review Fantasma { when sensor_inexistente < 30% -> reclassify_as_equilibrium }",
    );
    b.ticks(4);
    // a regra jamais avaliou: forma permanece nonequilibrium
    assert_eq!(
        b.engine.forma("Fantasma").unwrap().conjugation,
        vbl_lang::Conjugation::Nonequilibrium
    );
    assert!(!b.tem(kinds::TRANSICAO));
    assert!(b.tem_com(
        "ALERTA",
        &[("motivo", Json::str("sensor_nao_registrado")), ("sensor", Json::str("sensor_inexistente"))]
    ));
}

#[test]
fn sensor_ausente_nao_e_tratado_como_zero() {
    let mut b = montar(sim_padrao(), SENTINELA);
    b.engine.fxp.desregistrar_sensor("attention");
    b.ticks(3);
    assert!(!b.tem(kinds::TRANSICAO)); // nenhum disparo falso
    assert_eq!(
        b.engine.forma("Sentinela").unwrap().conjugation,
        vbl_lang::Conjugation::Nonequilibrium
    );
}

#[test]
fn sensor_registrado_inacessivel_segue_a_mesma_regra() {
    let mut b = montar(sim_padrao(), SENTINELA);
    b.engine.fxp.falhar_sensor("attention");
    b.tick();
    assert!(!b.tem(kinds::TRANSICAO));
    assert!(b.tem_com(
        "ALERTA",
        &[("motivo", Json::str("sensor_inacessivel")), ("sensor", Json::str("attention"))]
    ));
    // recupera a acessibilidade e a regra volta a avaliar
    b.engine.fxp.recuperar_sensor("attention");
    b.set_sensor("attention", 15.0);
    b.tick();
    assert!(b.tem_com(
        kinds::TRANSICAO,
        &[("forma", Json::str("Sentinela")), ("para", Json::str("equilibrium"))]
    ));
}

#[test]
fn forma_sem_source_path_nao_gera_leitura_nem_falha() {
    let mut b = montar(sim_padrao(), "event Piscada { value: \"impulso_curto\", horizon: 2s }");
    b.tick();
    assert!(b.forma_viva("Piscada")); // sem crash, sem leitura
    assert!(!b.tem_com("ALERTA", &[("motivo", Json::str("sensor_nao_registrado"))]));
}

// ======================================================================
// Cláusulas de erro em runtime (test_clausulas_erro.py, camada runtime)
// ======================================================================
#[test]
fn reclassify_para_nonequilibrium_sem_deadline_e_erro_registrado() {
    let mut b = montar(
        sim_padrao(),
        "event Ev { value: \"conteudo\", horizon: 30s }\n\
         review Ev { when cpu_temp > 90°C -> reclassify_as_nonequilibrium }",
    );
    b.set_sensor("cpu_temp", 95.0);
    b.tick();
    assert!(b.tem(kinds::RECLASSIFY_SEM_DEADLINE));
    assert_eq!(
        b.engine.forma("Ev").unwrap().conjugation,
        vbl_lang::Conjugation::Event
    );
}

#[test]
fn reclassify_nonequilibrium_preserva_deadline_declarado() {
    // NEQ -> EQ -> NEQ revive com o deadline DECLARADO original (FORMAL §3)
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium P { value: \"v\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s, exchange_mode: \"extraction\" }\n\
         review P { when attention < 30% -> reclassify_as_equilibrium,\n\
                    when attention > 80% -> reclassify_as_nonequilibrium }",
    );
    b.set_sensor("attention", 15.0);
    b.tick(); // NEQ -> EQ
    b.set_sensor("attention", 90.0);
    b.tick(); // EQ -> NEQ (deadline 3s declarado preservado)
    assert!(b.tem_com(
        kinds::TRANSICAO,
        &[("forma", Json::str("P")), ("para", Json::str("nonequilibrium"))]
    ));
    let form = b.engine.forma("P").unwrap();
    assert_eq!(form.conjugation, vbl_lang::Conjugation::Nonequilibrium);
    assert_eq!(form.manutencao.as_ref().unwrap().deadline_s, 3.0);
}

// ======================================================================
// Persistência (FORMAL §4.1) — `.vl` canônico + recarga
// ======================================================================
#[test]
fn reclassify_persiste_vl_canonico_reparseavel_com_sha256() {
    let dir = std::env::temp_dir().join(format!(
        "vbl-persist-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let sha;
    let texto_persistido;
    {
        let mut b = montar_em(
            sim_padrao(),
            "nonequilibrium Pensar { value: \"ideia\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s, exchange_mode: \"cooperation\" }\n\
             review Pensar { when attention < 30% -> reclassify_as_equilibrium }",
            &dir,
        );
        b.set_sensor("attention", 15.0);
        b.tick();
        sha = b
            .engine
            .caderno
            .buscar(kinds::PERSISTENCIA, &[("forma", Json::str("Pensar"))])
            .iter()
            .map(|e| match &e.extra {
                Json::Obj(c) => match c.get("sha256") {
                    Some(Json::Str(s)) => s.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            })
            .next()
            .unwrap();
        assert_eq!(sha.len(), 64);
        assert!(b.forma_viva("Pensar"));
        texto_persistido = std::fs::read_to_string(dir.join("Pensar.vl")).unwrap();
    }
    // o arquivo gravado é reparseável e reproduz o SHA registrado
    let texto = texto_persistido;
    let (_, diags) = vbl_lang::parse(&texto);
    assert!(!diags.has_errors(), "persistido não reparseou: {diags}\n{texto}");
    assert_eq!(vbl_runtime::caderno::sha256_hex(texto.as_bytes()), sha);
    // a forma persistida é a pós-transição: equilibrium (FORMAL §4.1),
    // com source_path preservado e horizon absoluto (60s, não renovado)
    assert!(texto.contains("equilibrium Pensar"), "{texto}");
    assert!(texto.contains("source_path: \"attention\""), "{texto}");
    assert!(texto.contains("horizon: 60s"), "{texto}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inicializacao_recarrega_equilibrium_com_horizon_nao_vencido() {
    let dir = std::env::temp_dir().join(format!(
        "vbl-recarga-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // simula uma persistência anterior (horizon não vencido)
    std::fs::write(
        dir.join("Doc.vl"),
        "equilibrium Doc {\n    value: \"documento\",\n    horizon: 100s,\n    cost_bytes: 64\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("Doc.json"), "{ \"creation_time\": 0 }\n").unwrap();
    // e uma expirada: criada 10s antes da época do programa, horizon 1s
    std::fs::write(
        dir.join("Velho.vl"),
        "equilibrium Velho {\n    value: \"antigo\",\n    horizon: 1s\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("Velho.json"), "{ \"creation_time\": -10 }\n").unwrap();

    let fxp = sim_padrao();
    let mut engine = Engine::novo(fxp, 1.0, &dir);
    let recarregadas = vbl_runtime::persist::recarregar_equilibrium(&mut engine);
    assert_eq!(recarregadas, 1);
    assert!(engine.forma("Doc").is_some());
    assert!(engine.forma("Velho").is_none());
    assert_eq!(engine.forma("Doc").unwrap().cost_bytes, Some(64));
    assert!(engine.caderno.tem_com("INFO", &[("motivo", Json::str("recarga"))]));
    assert!(engine.caderno.verify_chain());
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Escalonador — invariante de prazos (min-heap)
// ======================================================================
#[test]
fn escalonador_mantem_minheap_por_prazo() {
    let mut s = vbl_runtime::scheduler::Scheduler::new();
    s.agendar("A", Prazo::Horizon, 5.0, 1);
    s.agendar("B", Prazo::Horizon, 2.0, 2);
    s.agendar("C", Prazo::Manutencao, 7.0, 3);
    assert_eq!(s.proximo().unwrap().forma, "B");
    let vencidos = s.drenar_vencidos(3.0);
    assert_eq!(vencidos.len(), 1);
    assert_eq!(vencidos[0].forma, "B");
    assert_eq!(s.proximo().unwrap().forma, "A");
    s.remover_forma("A");
    assert_eq!(s.proximo().unwrap().forma, "C");
}

#[test]
fn heap_nao_cresce_ao_renovar_manutencao() {
    // keep implícito por 50 ticks: entradas vencidas são consumidas — o
    // tamanho do heap permanece limitado (1 horizon + 1 manutenção + ruído)
    let mut b = montar(
        sim_padrao(),
        "nonequilibrium V { value: \"v\", horizon: 1000s, source_path: \"cpu_power\", maintenance_deadline: 2s }\n\
         review V { when cpu_temp > 999°C -> dissolve }",
    );
    b.ticks(50);
    assert!(b.forma_viva("V"));
    assert!(b.engine.scheduler.len() <= 8, "heap cresceu: {}", b.engine.scheduler.len());
}

// ======================================================================
// Cenário BDD Caso 1 reancorado (fadiga de atenção) — ponta a ponta
// ======================================================================
#[test]
fn caso1_fadiga_de_atencao_ponta_a_ponta() {
    let dir = std::env::temp_dir().join(format!(
        "vbl-caso1-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    {
        let mut b = montar_em(
            sim_padrao(),
            "nonequilibrium PensarLivre { value: \"consciencia_anteneoliberal_ativa\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s, exchange_mode: \"cooperation\" }\n\
             review PensarLivre { when attention < 30% -> reclassify_as_equilibrium }",
            &dir,
        );
        b.set_sensor("attention", 15.0); // atenção esgotada
        b.tick();
        // transição gravada + persistência com SHA-256 + manutenção cessada
        assert!(b.tem_com(
            kinds::TRANSICAO,
            &[("forma", Json::str("PensarLivre")), ("para", Json::str("equilibrium"))]
        ));
        assert!(b.tem(kinds::PERSISTENCIA));
        assert_eq!(b.engine.forma("PensarLivre").unwrap().manutencao, None);
        assert!(!b.engine.retencao.labor.contains_key("PensarLivre")); // 0 bytes de trabalho
        assert!(b.engine.caderno.verify_chain());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Etapa 5 — decisão AD pendente da Etapa 4 (docs/ETAPA-4-RELATORIO.md §7):
// as regras de revisão SOBREVIVEM à reclassificação e permanecem ativas na
// `equilibrium`. Fundamento na FORMAL: o diagrama de estados (§4.1) lista
// `revisão` como caminho de EQ → DIS e a §4.2 manda avaliar as condições de
// revisão para CADA forma ativa — nada as desliga na `equilibrium`. O que
// cessa é a MANUTENÇÃO implícita (a conjugação não recebe ticks de
// manutenção — BDD Caso 1). Disparo sobre forma já na conjugação-alvo é
// no-op AUDITADO (AVALIACAO "já equilibrium"), sem nova transição.
// ======================================================================
#[test]
fn regras_permanecem_ativas_na_equilibrium_decisao_ad() {
    let mut b = montar(sim_padrao(), SENTINELA);
    b.set_sensor("attention", 15.0);
    b.tick(); // regra dispara: NEQ → EQ (persistida)
    assert!(b.tem_com(
        kinds::TRANSICAO,
        &[("forma", Json::str("Sentinela")), ("para", Json::str("equilibrium"))]
    ));
    assert_eq!(
        b.engine.forma("Sentinela").unwrap().conjugation,
        vbl_lang::Conjugation::Equilibrium
    );

    // ticks seguintes: a regra continua avaliada (condição segue verdadeira)
    // e o disparo é no-op auditado — sem segunda transição, sem dissolução
    b.ticks(3);
    assert_eq!(
        b.engine.forma("Sentinela").unwrap().conjugation,
        vbl_lang::Conjugation::Equilibrium,
        "a forma permanece equilibrium — no-op não reclassifica nem dissolve"
    );
    assert!(b.tem_com(
        "AVALIACAO",
        &[("forma", Json::str("Sentinela")), ("de", Json::str("equilibrium"))]
    ));

    // a manutenção implícita cessa: sem prazo de manutenção e sem trabalho
    // retido (FORMAL §4.1; BDD Caso 1: "deixa de receber ticks de manutenção")
    assert_eq!(b.engine.forma("Sentinela").unwrap().manutencao, None);
    assert!(!b.engine.retencao.labor.contains_key("Sentinela"));
}

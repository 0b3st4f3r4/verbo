//! O engine — governo do movimento (FORMAL §4.2).
//!
//! Loop por tick (relógio virtual injetável; 1 tick ≈ 1 s virtual):
//! 1. o mundo avança (`fxp.on_tick`) e o escalonador drena os prazos vencidos;
//! 2. para cada forma ativa (ordem de declaração):
//!    - vazamento energético (potência global repartida P/N × tick);
//!    - leitura do `source_path` (registro no Caderno; falha → alerta §4.7);
//!    - regras de revisão **na ordem declarada**, antes dos prazos
//!      (`review_short_circuit` sem revogar atuações despachadas);
//!    - prazos de manutenção e horizon — só se a forma seguir ativa.
//!
//! Semântica pinada pela suíte da Etapa 1: `horizon` absoluto (`>=`), colapso
//! no primeiro vencimento estrito (`>`), manutenção implícita enquanto houver
//! regra ativa, `subvert` dissolve no mesmo tick sem cancelar as ações
//! seguintes da própria regra, `reclassify_as_nonequilibrium` sem deadline
//! declarado = erro de runtime registrado, `notify_shutdown` não dissolve.

use crate::ledger::{kinds, Ledger, ChainLedger};
use crate::form::{ActionRt, Form, Maintenance, Retention, CANONICAL_POETIC_VALUE};
use crate::fxp::{Fxp, SensorFailure, PRIORITY_NORMAL, PRIORITY_SUBVERT, Value};
use crate::scheduler::{Deadline, Scheduler};
use std::collections::BTreeMap;
use vbl_lang::Conjugation;

/// Engine de tick. O barramento FXP é injetado (simulador/mock — FORMAL §4.2)
/// e o Caderno é injetável pelo mesmo trait (Etapa 4: produção assíncrona
/// sem mudar o runtime; default: [`ChainLedger`] em memória).
pub struct Engine<F: Fxp, C: Ledger = ChainLedger> {
    pub fxp: F,
    pub ledger: C,
    forms: BTreeMap<String, Form>,
    /// Ordem de declaração das formas ativas (iteração do tick). Etapa 5:
    /// `Rc<str>` — a iteração clona o ponteiro (incremento de contador, sem
    /// alocação) e itera um nome DONO, sem emprestar `self` (o corpo do tick
    /// muta campos e chama métodos `&mut self` livremente).
    order: Vec<std::rc::Rc<str>>,
    pub scheduler: Scheduler,
    pub sim_time: f64,
    pub clock: u64,
    tick_seconds: f64,
    persistence_dir: std::path::PathBuf,
    pub retention: Retention,
    /// Prazos vencidos no tick corrente (drenados do escalonador).
    due: BTreeMap<String, Vec<(Deadline, u64)>>,
}

impl<F: Fxp> Engine<F, ChainLedger> {
    pub fn new(fxp: F, tick_seconds: f64, persistence_dir: impl Into<std::path::PathBuf>) -> Self {
        Self::with_ledger(fxp, tick_seconds, persistence_dir, ChainLedger::new())
    }
}

impl<F: Fxp, C: Ledger> Engine<F, C> {
    /// Engine com Caderno injetado (Etapa 4 — Caderno de produção).
    pub fn with_ledger(
        fxp: F,
        tick_seconds: f64,
        persistence_dir: impl Into<std::path::PathBuf>,
        ledger: C,
    ) -> Self {
        Self {
            fxp,
            ledger,
            forms: BTreeMap::new(),
            order: Vec::new(),
            scheduler: Scheduler::new(),
            sim_time: 0.0,
            clock: 0,
            tick_seconds,
            persistence_dir: persistence_dir.into(),
            retention: Retention::default(),
            due: BTreeMap::new(),
        }
    }

    /// Propaga o relógio virtual para o Caderno sem avançar o mundo
    /// (usado antes de ticks externos ao loop principal).
    pub fn set_ledger_time(&mut self, tick: u64, t: f64) {
        self.ledger.set_time(tick, t);
    }

    pub fn persistence_dir(&self) -> &std::path::Path {
        &self.persistence_dir
    }

    /// Duração do tick virtual em segundos.
    pub fn tick_seconds(&self) -> f64 {
        self.tick_seconds
    }

    pub fn form(&self, name: &str) -> Option<&Form> {
        self.forms.get(name)
    }

    pub fn form_mut(&mut self, name: &str) -> Option<&mut Form> {
        self.forms.get_mut(name)
    }

    pub fn active_forms(&self) -> Vec<&Form> {
        self.order.iter().filter_map(|n| self.forms.get(n.as_ref())).collect()
    }

    pub fn active_names(&self) -> &[std::rc::Rc<str>] {
        &self.order
    }

    // ------------------------------------------------------------------
    // Registro e dissolução
    // ------------------------------------------------------------------
    pub fn register_form(&mut self, mut form: Form) {
        form.horizon_version += 1;
        if form.conjugation == Conjugation::Nonequilibrium {
            form.maintenance_version += 1;
            let mode = form.exchange_mode.clone().unwrap_or_else(|| "cooperation".into());
            let canonical = mode == "cooperation" || mode == "extraction";
            if !canonical {
                self.ledger.alert(
                    &format!(
                        "exchange_mode '{mode}' da forma '{}' não é canônico (cooperation|extraction).",
                        form.name
                    ),
                    Json::obj([
                        ("motivo", Json::str("exchange_mode_nao_canonico")),
                        ("forma", Json::str(&form.name)),
                        ("exchange_mode", Json::str(&mode)),
                    ]),
                );
            }
            // `exchange_mode` — anotação de auditoria (PLAN §2.2: o efeito
            // semântico pleno permanece em definição; default registrado).
            self.ledger.info(
                &format!(
                    "exchange_mode da forma '{}' = '{mode}' (anotação de auditoria).",
                    form.name
                ),
                Json::obj([
                    ("forma", Json::str(&form.name)),
                    ("exchange_mode", Json::str(&mode)),
                    ("efeito", Json::str("anotacao_auditoria")),
                ]),
            );
        }
        self.ledger.info(
            &format!("Forma '{}' conjugada no sistema.", form.name),
            Json::obj([("forma", Json::str(&form.name))]),
        );
        self.bind(form);
    }

    /// Registra a forma ativa e agenda seus prazos (O(log N) por prazo).
    /// Etapa 5: o teste de presença consulta o mapa de formas (O(log N)) —
    /// `order` e `forms` têm sempre os MESMOS nomes (invariante mantido por
    /// `bind`/`dissolve_form`), o que torna o `contains` O(N) redundante.
    fn bind(&mut self, form: Form) {
        let bytes = form.bytes_retidos();
        let name = form.name.clone();
        let at_horizon = form.horizon_end();
        let h_version = form.horizon_version;
        let neq = form.conjugation == Conjugation::Nonequilibrium;
        let m_version = form.maintenance_version;
        let mut mentry = None;
        if let Some(m) = &form.maintenance {
            mentry = Some(m.last + m.deadline_s);
        }
        if !self.forms.contains_key(&name) {
            self.order.push(std::rc::Rc::from(name.as_str()));
        }
        self.retention.per_form.insert(name.clone(), bytes);
        if neq {
            self.retention.labor.insert(name.clone(), bytes + 24);
        } else {
            self.retention.labor.remove(&name);
        }
        self.forms.insert(name.clone(), form);
        self.scheduler.schedule(&name, Deadline::Horizon, at_horizon, h_version);
        if let Some(em) = mentry {
            self.scheduler.schedule(&name, Deadline::Maintenance, em, m_version);
        }
    }

    /// Dissolução com fim tipificado (FORMAL §6): recursos liberados no
    /// mesmo tick (contadores de retenção → 0).
    pub fn dissolve_form(&mut self, name: &str, end: &str) {
        if self.forms.remove(name).is_some() {
            self.order.retain(|n| n.as_ref() != name);
            self.retention.per_form.remove(name);
            self.retention.labor.remove(name);
            self.scheduler.remove_form(name);
            self.ledger.record(
                end,
                &format!("Forma '{name}' dissolvida ({end})."),
                Json::obj([("forma", Json::str(name))]),
            );
            self.ledger.info(
                &format!("ALÍVIO TERMODINÂMICO -> Forma '{name}' dissolvida."),
                Json::obj([("forma", Json::str(name))]),
            );
        }
    }

    /// FORMAL §4.5: substitui o valor pelo poético canônico e marca a forma
    /// para dissolução no MESMO tick. Sem efeito físico no runtime — reações
    /// do mundo são roteirizadas pelo FXP/cenário.
    fn subvert_form(&mut self, name: &str) {
        if let Some(form) = self.forms.get_mut(name) {
            form.value = Value::Str(CANONICAL_POETIC_VALUE.into());
            self.ledger.art(
                &format!("Operador subvert() invocado na forma '{name}'! Acumulação abortada."),
                Json::obj([("forma", Json::str(name))]),
            );
            self.ledger.record(
                kinds::SUBVERT_APPLIED,
                &format!("Novo valor de '{name}': '{CANONICAL_POETIC_VALUE}'"),
                Json::obj([
                    ("forma", Json::str(name)),
                    ("novo_valor", Json::str(CANONICAL_POETIC_VALUE)),
                ]),
            );
        }
    }

    // ------------------------------------------------------------------
    // Tick
    // ------------------------------------------------------------------
    pub fn tick(&mut self) {
        self.clock += 1;
        self.sim_time += self.tick_seconds;
        let now = self.sim_time;
        // Etapa 4 (AGENTS §1.4): todo evento carrega o relógio virtual.
        self.ledger.set_time(self.clock, self.sim_time);

        // 0. o mundo avança (roteirização do simulador)
        self.fxp.on_tick(&mut self.ledger);
        // Etapa 4: relógio virtual + potência global propagados ao Caderno
        // (AGENTS §1.4 — timestamps; PLAN §4.1 — custo de atuações)
        let global_power = self.fxp.cpu_power();
        self.ledger.set_time(self.clock, self.sim_time);
        self.ledger.set_power(global_power);

        // 1. drena os prazos vencidos do escalonador (O(vencidos))
        self.due.clear();
        for entry in self.scheduler.drain_due(now) {
            self.due
                .entry(entry.form)
                .or_default()
                .push((entry.deadline, entry.version));
        }

        // 2. iteração por ÍNDICE sobre a ordem (Etapa 5 — sem snapshot de
        //    `String`s por tick): no caminho comum toda mutação é de CAMPO
        //    disjunto de `order`, e o nome itera emprestado (zero alocação).
        //    As dissoluções encolhem `order` na posição corrente e `continue`m
        //    SEM avançar o índice — o elemento seguinte desloca para a
        //    posição liberada; o divisor P/N usa o total ANTES do tick
        //    (mesma semântica do instantâneo da Etapa 1).
        let total_active = self.order.len();
        let mut i = 0usize;
        // condição VIVA (`self.order.len()`): uma dissolução encolhe a ordem
        // na posição corrente e o `continue` sem incrementar processa o
        // elemento deslocado — `total_active` (obsoleto) só divide a potência.
        while i < self.order.len() {
            // clone do ponteiro (contador de referência — sem alocação); o
            // nome iterado é DONO local: nenhuma parte do corpo empresta
            // `self.order`, e as mutações de campos/chamadas `&mut self`
            // permanecem livres de conflito de borrow.
            let name_pointer: std::rc::Rc<str> = self.order[i].clone();
            let name: &str = &name_pointer;
            if !self.forms.contains_key(name) {
                i += 1;
                continue;
            }
            let due_forms = self.due.remove(name).unwrap_or_default();

            // 2a. vazamento energético — partilha igual P/N (FORMAL §4.2)
            let power = if total_active > 0 {
                global_power / total_active as f64
            } else {
                0.0
            };
            self.ledger.leak(name, power, self.tick_seconds);

            // 2b. leitura do sensor principal (source_path) — formas sem
            //     source_path não geram leitura nem falha (FORMAL §4.7)
            let source = self.forms.get(name).and_then(|f| f.source_path.as_deref());
            if let Some(sensor) = source {
                match self.fxp.read_sensor(sensor, &mut self.ledger) {
                    Ok(v) => self.ledger.sensor_read(sensor, v),
                    Err(_) => { /* alerta já registrado pelo FXP (§4.7) */ }
                }
            }

            // 2c. regras de revisão, na ordem declarada (FORMAL §4.2) —
            //     Etapa 5: iteração por índice SEM clonar a tabela de regras
            //     por forma/tick; o clone fica restrito às actions do
            //     disparo (caminho frio).
            let mut fired = false;
            let rule_count = self.forms.get(name).map(|f| f.rules.len()).unwrap_or(0);
            for index in 0..rule_count {
                let Some(rule) = self.forms.get(name).and_then(|f| f.rules.get(index)) else {
                    break;
                };
                let reading = self.fxp.read_sensor(&rule.sensor, &mut self.ledger);
                let sensor_value = match reading {
                    Ok(v) => v,
                    Err(SensorFailure::NotRegistered) | Err(SensorFailure::Inaccessible) => {
                        // falha de I/O já alertada; condição NÃO avaliada (§4.7)
                        continue;
                    }
                };
                self.ledger.sensor_read(&rule.sensor, sensor_value);
                if rule.op.evaluate(sensor_value, rule.threshold) {
                    self.ledger.alert(
                        &format!(
                            "Condição de revisão disparada para '{name}': {} {} {} (lido: {sensor_value})",
                            rule.sensor,
                            rule.op.symbol(),
                            fmt_threshold(rule.threshold)
                        ),
                        Json::obj([
                            ("forma", Json::str(name)),
                            ("sensor", Json::str(&rule.sensor)),
                        ]),
                    );
                    let actions = rule.actions.clone();
                    if self.execute_actions(name, &actions) {
                        fired = true;
                        let remaining = rule_count - index - 1;
                        if remaining > 0 {
                            // review_short_circuit: regras seguintes da mesma
                            // review não são avaliadas naquele tick — sem
                            // revogar atuações já despachadas (§4.2/§4.5)
                            self.ledger.record(
                                kinds::REVIEW_SHORT_CIRCUIT,
                                &format!(
                                    "'{name}': {remaining} regra(s) seguinte(s) não avaliada(s) neste tick."
                                ),
                                Json::obj([
                                    ("forma", Json::str(name)),
                                    ("regras_restantes", Json::num(remaining as f64)),
                                ]),
                            );
                        }
                        break;
                    }
                }
            }
            if fired || !self.forms.contains_key(name) {
                if self.forms.contains_key(name) {
                    // prazos pulados neste tick voltam para o próximo (o
                    // estado decide — mesmo contrato do protótipo da Etapa 1;
                    // Etapa 5: forma DISSOLVIDA não re-agenda — nada de lixo
                    // de prazos órfãos no heap do escalonador)
                    for (deadline, version) in &due_forms {
                        self.scheduler.schedule(name, *deadline, now + self.tick_seconds, *version);
                    }
                    // reclassificação (ordem intacta): próxima forma
                    i += 1;
                }
                // senão: dissolvida/subvertida neste tick — `order` encolheu
                // na posição corrente; NÃO avançar o índice
                continue;
            }

            // versões vivas (validade das entradas do escalonador)
            let (is_neq, has_rules, m_alive, h_alive) = self
                .forms
                .get(name)
                .map(|f| {
                    (
                        f.conjugation == Conjugation::Nonequilibrium,
                        !f.rules.is_empty(),
                        f.maintenance_version,
                        f.horizon_version,
                    )
                })
                .unwrap_or((false, false, 0, 0));

            // marca de dissolução NESTE tick: `order` encolheu na posição
            // corrente → `continue` SEM avançar o índice (o corpo termina em
            // `if dissolved { continue; } i += 1;`)
            let mut dissolved = false;

            // 2d. manutenção (apenas nonequilibrium) — keep implícito com
            //     regra ativa; colapso no primeiro vencimento estrito sem regra
            if is_neq {
                if has_rules {
                    // manutenção implícita (FORMAL §4.1 ii)
                    let (version, deadline) = {
                        let f = self.forms.get_mut(name).unwrap();
                        f.keep(now);
                        f.maintenance_version += 1;
                        (
                            f.maintenance_version,
                            f.maintenance.as_ref().map(|m| m.last + m.deadline_s).unwrap_or(now),
                        )
                    };
                    self.scheduler.schedule(name, Deadline::Maintenance, deadline, version);
                } else {
                    let deadline_expired = due_forms.iter().any(|(p, _)| *p == Deadline::Maintenance);
                    let due_now = self
                        .forms
                        .get(name)
                        .map(|f| f.maintenance_due(now))
                        .unwrap_or(false);
                    if deadline_expired && due_now {
                        let deadline_s = self
                            .forms
                            .get(name)
                            .and_then(|f| f.maintenance.as_ref())
                            .map(|m| m.deadline_s)
                            .unwrap_or(0.0);
                        self.ledger.collapse(
                            &format!(
                                "Prazo de manutenção de '{name}' expirou! (sem keep() por {deadline_s}s)"
                            ),
                            Json::obj([("forma", Json::str(name))]),
                        );
                        self.dissolve_form(name, kinds::COLLAPSE_MAINTENANCE);
                        dissolved = true;
                    }
                    if !dissolved {
                        // limite exato ainda sustenta: reagenda o prazo atual
                        for (deadline, version) in &due_forms {
                            if *deadline == Deadline::Maintenance && *version == m_alive {
                                self.scheduler
                                    .schedule(name, *deadline, now + self.tick_seconds, *version);
                            }
                        }
                    }
                }
            }

            // 2e. horizon — apenas se a forma seguir ativa (FORMAL §4.2)
            if !dissolved {
                let horizon_expired = due_forms.iter().any(|(p, _)| *p == Deadline::Horizon);
                let exhausted_now = self
                    .forms
                    .get(name)
                    .map(|f| f.horizon_exhausted(now))
                    .unwrap_or(false);
                if horizon_expired && exhausted_now {
                    self.ledger.warn(
                        &format!("Horizonte de validade de '{name}' esgotou-se. Dissolvendo."),
                        Json::obj([("forma", Json::str(name))]),
                    );
                    self.dissolve_form(name, kinds::DISSOLVE_HORIZON);
                    dissolved = true;
                } else if horizon_expired {
                    // borda de arredondamento: reagenda para o próximo tick
                    for (deadline, version) in &due_forms {
                        if *deadline == Deadline::Horizon && *version == h_alive {
                            self.scheduler
                                .schedule(name, *deadline, now + self.tick_seconds, *version);
                        }
                    }
                }
            }

            if dissolved {
                continue; // `order` encolheu na posição corrente — sem avançar
            }
            i += 1;
        }
    }

    // ------------------------------------------------------------------
    // Ações (FORMAL §4.2/§4.5/§4.6)
    // ------------------------------------------------------------------
    /// Executa a action_list na ordem declarada. Devolve true se a forma
    /// deixou de existir na conjugação anterior (dissolvida/reclassificada).
    fn execute_actions(&mut self, name: &str, actions: &[ActionRt]) -> bool {
        if !self.forms.contains_key(name) {
            // ação de revisão sobre forma já dissolvida no mesmo tick é
            // ignorada, com registro (FORMAL §4.1)
            self.ledger.record(
                kinds::REVIEW_AFTER_DISSOLUTION,
                &format!("Ação de revisão sobre '{name}' ignorada: forma já dissolvida neste tick."),
                Json::obj([("forma", Json::str(name))]),
            );
            return true;
        }
        let mut doomed = false;
        for action in actions {
            match action {
                ActionRt::Dissolve => {
                    self.dissolve_form(name, kinds::DISSOLVE_RULE);
                    return true;
                }
                ActionRt::Subvert => {
                    // §4.5: subvert não cancela as ações seguintes da mesma
                    // regra — em particular, qualquer act associado é enviado
                    self.subvert_form(name);
                    doomed = true;
                }
                ActionRt::ReclassifyEquilibrium => {
                    self.reclassify_equilibrium(name);
                    return true;
                }
                ActionRt::ReclassifyNonequilibrium => {
                    if self.reclassify_nonequilibrium(name) {
                        return true;
                    }
                }
                ActionRt::NotifyShutdown => {
                    // §4.6: não dissolve, não interrompe as ações seguintes
                    self.ledger.warn(
                        &format!(
                            "Interrupção do sistema! Desligando cargas secundárias ligadas a '{name}'."
                        ),
                        Json::obj([("forma", Json::str(name))]),
                    );
                }
                ActionRt::Act { actor, value } => {
                    // Etapa 3 (FORMAL §4.5): act na mesma regra após subvert
                    // entra na fila do FXP com prioridade máxima.
                    let priority = if doomed { PRIORITY_SUBVERT } else { PRIORITY_NORMAL };
                    let outcome = self
                        .fxp
                        .act_with_priority(actor, value.clone(), priority, &mut self.ledger);
                    if !outcome.ok() {
                        self.ledger.alert(
                            &format!("Falha na atuação do ator '{actor}' para a forma '{name}'."),
                            Json::obj([
                                ("forma", Json::str(name)),
                                ("ator", Json::str(actor)),
                                ("outcome", Json::str(format!("{outcome:?}"))),
                            ]),
                        );
                    }
                }
            }
        }
        if doomed {
            // dissolução da forma subvertida dentro do mesmo tick (§4.5)
            self.dissolve_form(name, kinds::DISSOLVE_SUBVERT);
            return true;
        }
        false
    }

    /// `event→equilibrium` e `nonequilibrium→equilibrium` (FORMAL §4.1):
    /// persiste em disco (`.vl` canônico + SHA-256) e converte.
    /// `equilibrium→equilibrium` não é transição da matriz — no-op auditado.
    fn reclassify_equilibrium(&mut self, name: &str) {
        let form = match self.forms.get(name) {
            Some(f) => f.clone(),
            None => return,
        };
        if form.conjugation == Conjugation::Equilibrium {
            self.ledger.warn(
                &format!(
                    "reclassify_as_equilibrium sobre '{name}' (já equilibrium) — sem efeito (matriz de transições, FORMAL §4.1)."
                ),
                Json::obj([("forma", Json::str(name)), ("de", Json::str("equilibrium"))]),
            );
            return;
        }
        // horizon ABSOLUTO: creation_time original é preservado (§4.1)
        let mut new = form.clone();
        new.conjugation = Conjugation::Equilibrium;
        new.currency = Conjugation::Equilibrium.default_currency().into();
        new.maintenance = None;
        new.maintenance_version = 0;
        new.cost_bytes = None; // tamanho real gravado (FORMAL §4.1)
        new.horizon_version += 1;

        self.ledger.record(
            kinds::TRANSITION,
            &format!("Forma '{name}' reclassificada para 'equilibrium' (persistida)."),
            Json::obj([
                ("forma", Json::str(name)),
                ("de", Json::str(form.conjugation.name())),
                ("para", Json::str("equilibrium")),
            ]),
        );
        self.bind(new.clone());
        if let Err(e) = self.persist(&new) {
            self.ledger.alert(
                &format!("Falha ao persistir '{name}': {e}"),
                Json::obj([
                    ("forma", Json::str(name)),
                    ("motivo", Json::str("persistencia_falhou")),
                ]),
            );
        }
    }

    /// `equilibrium→nonequilibrium` e `nonequilibrium→nonequilibrium` (keep).
    /// Sem deadline declarado: erro de runtime registrado — a forma permanece
    /// (FORMAL §3).
    fn reclassify_nonequilibrium(&mut self, name: &str) -> bool {
        let form = match self.forms.get(name) {
            Some(f) => f.clone(),
            None => return false,
        };
        let Some(deadline) = form.declared_maintenance_deadline else {
            self.ledger.record(
                kinds::RECLASSIFY_NO_DEADLINE,
                &format!(
                    "reclassify_as_nonequilibrium recusado para '{name}': sem maintenance_deadline declarado (FORMAL §3). A forma permanece como estava."
                ),
                Json::obj([("forma", Json::str(name))]),
            );
            return true; // a conjugação "tentou mudar" → short circuit da review
        };
        let mode = form.exchange_mode.clone().unwrap_or_else(|| "cooperation".into());
        let mut new = form.clone();
        new.conjugation = Conjugation::Nonequilibrium;
        new.currency = Conjugation::Nonequilibrium.default_currency().into();
        new.maintenance = Some(Maintenance {
            deadline_s: deadline,
            // semântica do protótipo: última manutenção parte da criação
            // original (horizon absoluto; keep implícito/regra renova em t+1)
            last: form.creation_time,
        });
        new.maintenance_version += 1;
        new.horizon_version += 1;
        new.exchange_mode = Some(mode);
        self.ledger.record(
            kinds::TRANSITION,
            &format!("Forma '{name}' reclassificada para 'nonequilibrium' (trabalho ativo)."),
            Json::obj([
                ("forma", Json::str(name)),
                ("de", Json::str(form.conjugation.name())),
                ("para", Json::str("nonequilibrium")),
            ]),
        );
        self.bind(new);
        true
    }

    // ------------------------------------------------------------------
    // Persistência (FORMAL §4.1): `.vl` canônico + SHA-256 no Caderno
    // ------------------------------------------------------------------
    fn persist(&mut self, form: &Form) -> Result<(String, String), String> {
        std::fs::create_dir_all(&self.persistence_dir)
            .map_err(|e| format!("diretório {}: {e}", self.persistence_dir.display()))?;
        let decl = form_to_ast(form);
        let content = vbl_lang::canon::form_to_vl(&decl);
        let data = content.as_bytes();
        let path = self.persistence_dir.join(format!("{}.vl", form.name));
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        let sha256 = crate::ledger::sha256_hex(data);
        let bytes = data.len() as u64;
        self.ledger.record(
            kinds::PERSISTENCE,
            &format!("Forma '{}' persistida como `.vl` canônico.", form.name),
            Json::obj([
                ("forma", Json::str(&form.name)),
                ("caminho", Json::str(path.display().to_string())),
                ("sha256", Json::str(&sha256)),
                ("bytes", Json::num(bytes as f64)),
            ]),
        );
        // cost_bytes ausente passa a valer o tamanho real gravado (FORMAL §4.1)
        if let Some(f) = self.forms.get_mut(&form.name) {
            if f.cost_bytes.is_none() {
                f.cost_bytes = Some(bytes);
            }
        }
        // sidecar: creation_time para recarregar com horizon absoluto íntegro
        let _ = crate::persist::write_sidecar(
            &self.persistence_dir,
            &form.name,
            form.creation_time,
        );
        self.fxp.add_disk_bytes(1024); // escrita simulada no suporte estável
        Ok((path.display().to_string(), sha256))
    }
}

/// Formata threshold para a mensagem do Caderno (inteiro sem casa decimal).
fn fmt_threshold(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

use crate::json::Json;

/// Converte a forma runtime de volta para AST (serialização canônica).
pub fn form_to_ast(form: &Form) -> vbl_lang::FormDecl {
    use vbl_lang::FormAttrs;
    let span = vbl_lang::Span::default();
    let value = match &form.value {
        Value::Num(n) => vbl_lang::Expression::num(*n, span),
        Value::Str(s) => vbl_lang::Expression::str(s.clone(), span),
        Value::Ident(s) => vbl_lang::Expression::ident(s.clone(), span),
    };
    let horizon = ast_duration(form.horizon_s, span);
    let maintenance_deadline = form.maintenance.as_ref().map(|m| ast_duration(m.deadline_s, span));
    let attrs = FormAttrs {
        source_path: form.source_path.clone(),
        maintenance_deadline: if form.conjugation == Conjugation::Nonequilibrium {
            maintenance_deadline
        } else {
            None
        },
        exchange_mode: if form.conjugation == Conjugation::Nonequilibrium {
            Some(form.exchange_mode.clone().unwrap_or_else(|| "cooperation".into()))
        } else {
            None
        },
        cost_bytes: if form.conjugation == Conjugation::Equilibrium {
            form.cost_bytes.map(|b| b as i128)
        } else {
            None
        },
        currency: if form.currency == form.conjugation.default_currency() {
            None
        } else {
            Some(form.currency.clone())
        },
        classification: form.classification.clone(),
    };
    vbl_lang::FormDecl {
        conjugation: form.conjugation,
        name: form.name.clone(),
        value,
        horizon,
        attrs,
        span,
    }
}

/// Duração AST a partir de segundos (escolhe a unidade canônica).
fn ast_duration(seconds: f64, span: vbl_lang::Span) -> vbl_lang::Duration {
    use vbl_lang::TimeUnit;
    let (value, unit) = if seconds.fract() == 0.0 {
        (seconds, TimeUnit::S)
    } else if (seconds * 1e3).fract().abs() < 1e-9 {
        (seconds * 1e3, TimeUnit::Ms)
    } else {
        (seconds, TimeUnit::S)
    };
    vbl_lang::Duration { value, unit, span }
}

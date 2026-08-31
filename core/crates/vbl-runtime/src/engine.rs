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

use crate::notebook::{kinds, Caderno, ChainCaderno};
use crate::form::{ActionRt, Form, Manutencao, Retencao, VALOR_POETICO_CANONICO};
use crate::fxp::{Fxp, FalhaSensor, PRIORIDADE_NORMAL, PRIORIDADE_SUBVERT, Value};
use crate::scheduler::{Prazo, Scheduler};
use std::collections::BTreeMap;
use vbl_lang::Conjugation;

/// Engine de tick. O barramento FXP é injetado (simulador/mock — FORMAL §4.2)
/// e o Caderno é injetável pelo mesmo trait (Etapa 4: produção assíncrona
/// sem mudar o runtime; default: [`ChainCaderno`] em memória).
pub struct Engine<F: Fxp, C: Caderno = ChainCaderno> {
    pub fxp: F,
    pub caderno: C,
    formas: BTreeMap<String, Form>,
    /// Ordem de declaração das formas ativas (iteração do tick). Etapa 5:
    /// `Rc<str>` — a iteração clona o ponteiro (incremento de contador, sem
    /// alocação) e itera um nome DONO, sem emprestar `self` (o corpo do tick
    /// muta campos e chama métodos `&mut self` livremente).
    ordem: Vec<std::rc::Rc<str>>,
    pub scheduler: Scheduler,
    pub sim_time: f64,
    pub clock: u64,
    tick_seconds: f64,
    persistence_dir: std::path::PathBuf,
    pub retencao: Retencao,
    /// Prazos vencidos no tick corrente (drenados do escalonador).
    vencidos: BTreeMap<String, Vec<(Prazo, u64)>>,
}

impl<F: Fxp> Engine<F, ChainCaderno> {
    pub fn novo(fxp: F, tick_seconds: f64, persistence_dir: impl Into<std::path::PathBuf>) -> Self {
        Self::com_caderno(fxp, tick_seconds, persistence_dir, ChainCaderno::new())
    }
}

impl<F: Fxp, C: Caderno> Engine<F, C> {
    /// Engine com Caderno injetado (Etapa 4 — Caderno de produção).
    pub fn com_caderno(
        fxp: F,
        tick_seconds: f64,
        persistence_dir: impl Into<std::path::PathBuf>,
        caderno: C,
    ) -> Self {
        Self {
            fxp,
            caderno,
            formas: BTreeMap::new(),
            ordem: Vec::new(),
            scheduler: Scheduler::new(),
            sim_time: 0.0,
            clock: 0,
            tick_seconds,
            persistence_dir: persistence_dir.into(),
            retencao: Retencao::default(),
            vencidos: BTreeMap::new(),
        }
    }

    /// Propaga o relógio virtual para o Caderno sem avançar o mundo
    /// (usado antes de ticks externos ao loop principal).
    pub fn definir_tempo_caderno(&mut self, tick: u64, t: f64) {
        self.caderno.definir_tempo(tick, t);
    }

    pub fn persistence_dir(&self) -> &std::path::Path {
        &self.persistence_dir
    }

    /// Duração do tick virtual em segundos.
    pub fn tick_seconds(&self) -> f64 {
        self.tick_seconds
    }

    pub fn forma(&self, nome: &str) -> Option<&Form> {
        self.formas.get(nome)
    }

    pub fn forma_mut(&mut self, nome: &str) -> Option<&mut Form> {
        self.formas.get_mut(nome)
    }

    pub fn formas_ativas(&self) -> Vec<&Form> {
        self.ordem.iter().filter_map(|n| self.formas.get(n.as_ref())).collect()
    }

    pub fn nomes_ativos(&self) -> &[std::rc::Rc<str>] {
        &self.ordem
    }

    // ------------------------------------------------------------------
    // Registro e dissolução
    // ------------------------------------------------------------------
    pub fn registrar_forma(&mut self, mut form: Form) {
        form.horizon_versao += 1;
        if form.conjugation == Conjugation::Nonequilibrium {
            form.manutencao_versao += 1;
            let modo = form.exchange_mode.clone().unwrap_or_else(|| "cooperation".into());
            let canonico = modo == "cooperation" || modo == "extraction";
            if !canonico {
                self.caderno.alert(
                    &format!(
                        "exchange_mode '{modo}' da forma '{}' não é canônico (cooperation|extraction).",
                        form.name
                    ),
                    Json::obj([
                        ("motivo", Json::str("exchange_mode_nao_canonico")),
                        ("forma", Json::str(&form.name)),
                        ("exchange_mode", Json::str(&modo)),
                    ]),
                );
            }
            // `exchange_mode` — anotação de auditoria (PLAN §2.2: o efeito
            // semântico pleno permanece em definição; default registrado).
            self.caderno.info(
                &format!(
                    "exchange_mode da forma '{}' = '{modo}' (anotação de auditoria).",
                    form.name
                ),
                Json::obj([
                    ("forma", Json::str(&form.name)),
                    ("exchange_mode", Json::str(&modo)),
                    ("efeito", Json::str("anotacao_auditoria")),
                ]),
            );
        }
        self.caderno.info(
            &format!("Forma '{}' conjugada no sistema.", form.name),
            Json::obj([("forma", Json::str(&form.name))]),
        );
        self.bind(form);
    }

    /// Registra a forma ativa e agenda seus prazos (O(log N) por prazo).
    /// Etapa 5: o teste de presença consulta o mapa de formas (O(log N)) —
    /// `ordem` e `formas` têm sempre os MESMOS nomes (invariante mantido por
    /// `bind`/`dissolve_form`), o que torna o `contains` O(N) redundante.
    fn bind(&mut self, form: Form) {
        let bytes = form.bytes_retidos();
        let nome = form.name.clone();
        let em_horizonte = form.horizonte_fim();
        let hversao = form.horizon_versao;
        let neq = form.conjugation == Conjugation::Nonequilibrium;
        let mversao = form.manutencao_versao;
        let mut mentry = None;
        if let Some(m) = &form.manutencao {
            mentry = Some(m.ultima + m.deadline_s);
        }
        if !self.formas.contains_key(&nome) {
            self.ordem.push(std::rc::Rc::from(nome.as_str()));
        }
        self.retencao.por_forma.insert(nome.clone(), bytes);
        if neq {
            self.retencao.labor.insert(nome.clone(), bytes + 24);
        } else {
            self.retencao.labor.remove(&nome);
        }
        self.formas.insert(nome.clone(), form);
        self.scheduler.agendar(&nome, Prazo::Horizon, em_horizonte, hversao);
        if let Some(em) = mentry {
            self.scheduler.agendar(&nome, Prazo::Manutencao, em, mversao);
        }
    }

    /// Dissolução com fim tipificado (FORMAL §6): recursos liberados no
    /// mesmo tick (contadores de retenção → 0).
    pub fn dissolve_form(&mut self, nome: &str, fim: &str) {
        if self.formas.remove(nome).is_some() {
            self.ordem.retain(|n| n.as_ref() != nome);
            self.retencao.por_forma.remove(nome);
            self.retencao.labor.remove(nome);
            self.scheduler.remover_forma(nome);
            self.caderno.record(
                fim,
                &format!("Forma '{nome}' dissolvida ({fim})."),
                Json::obj([("forma", Json::str(nome))]),
            );
            self.caderno.info(
                &format!("ALÍVIO TERMODINÂMICO -> Forma '{nome}' dissolvida."),
                Json::obj([("forma", Json::str(nome))]),
            );
        }
    }

    /// FORMAL §4.5: substitui o valor pelo poético canônico e marca a forma
    /// para dissolução no MESMO tick. Sem efeito físico no runtime — reações
    /// do mundo são roteirizadas pelo FXP/cenário.
    fn subvert_form(&mut self, nome: &str) {
        if let Some(form) = self.formas.get_mut(nome) {
            form.value = Value::Str(VALOR_POETICO_CANONICO.into());
            self.caderno.art(
                &format!("Operador subvert() invocado na forma '{nome}'! Acumulação abortada."),
                Json::obj([("forma", Json::str(nome))]),
            );
            self.caderno.record(
                kinds::SUBVERT_APLICADO,
                &format!("Novo valor de '{nome}': '{VALOR_POETICO_CANONICO}'"),
                Json::obj([
                    ("forma", Json::str(nome)),
                    ("novo_valor", Json::str(VALOR_POETICO_CANONICO)),
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
        let agora = self.sim_time;
        // Etapa 4 (AGENTS §1.4): todo evento carrega o relógio virtual.
        self.caderno.definir_tempo(self.clock, self.sim_time);

        // 0. o mundo avança (roteirização do simulador)
        self.fxp.on_tick(&mut self.caderno);
        // Etapa 4: relógio virtual + potência global propagados ao Caderno
        // (AGENTS §1.4 — timestamps; PLAN §4.1 — custo de atuações)
        let potencia_global = self.fxp.cpu_power();
        self.caderno.definir_tempo(self.clock, self.sim_time);
        self.caderno.definir_potencia(potencia_global);

        // 1. drena os prazos vencidos do escalonador (O(vencidos))
        self.vencidos.clear();
        for entrada in self.scheduler.drenar_vencidos(agora) {
            self.vencidos
                .entry(entrada.forma)
                .or_default()
                .push((entrada.prazo, entrada.versao));
        }

        // 2. iteração por ÍNDICE sobre a ordem (Etapa 5 — sem snapshot de
        //    `String`s por tick): no caminho comum toda mutação é de CAMPO
        //    disjunto de `ordem`, e o nome itera emprestado (zero alocação).
        //    As dissoluções encolhem `ordem` na posição corrente e `continue`m
        //    SEM avançar o índice — o elemento seguinte desloca para a
        //    posição liberada; o divisor P/N usa o total ANTES do tick
        //    (mesma semântica do instantâneo da Etapa 1).
        let total_ativas = self.ordem.len();
        let mut i = 0usize;
        // condição VIVA (`self.ordem.len()`): uma dissolução encolhe a ordem
        // na posição corrente e o `continue` sem incrementar processa o
        // elemento deslocado — `total_ativas` (obsoleto) só divide a potência.
        while i < self.ordem.len() {
            // clone do ponteiro (contador de referência — sem alocação); o
            // nome iterado é DONO local: nenhuma parte do corpo empresta
            // `self.ordem`, e as mutações de campos/chamadas `&mut self`
            // permanecem livres de conflito de borrow.
            let nome_ponteiro: std::rc::Rc<str> = self.ordem[i].clone();
            let nome: &str = &nome_ponteiro;
            if !self.formas.contains_key(nome) {
                i += 1;
                continue;
            }
            let vencidos_forma = self.vencidos.remove(nome).unwrap_or_default();

            // 2a. vazamento energético — partilha igual P/N (FORMAL §4.2)
            let potencia = if total_ativas > 0 {
                potencia_global / total_ativas as f64
            } else {
                0.0
            };
            self.caderno.leak(nome, potencia, self.tick_seconds);

            // 2b. leitura do sensor principal (source_path) — formas sem
            //     source_path não geram leitura nem falha (FORMAL §4.7)
            let source = self.formas.get(nome).and_then(|f| f.source_path.as_deref());
            if let Some(sensor) = source {
                match self.fxp.read_sensor(sensor, &mut self.caderno) {
                    Ok(v) => self.caderno.sensor_read(sensor, v),
                    Err(_) => { /* alerta já registrado pelo FXP (§4.7) */ }
                }
            }

            // 2c. regras de revisão, na ordem declarada (FORMAL §4.2) —
            //     Etapa 5: iteração por índice SEM clonar a tabela de regras
            //     por forma/tick; o clone fica restrito às actions do
            //     disparo (caminho frio).
            let mut disparou = false;
            let qtd_regras = self.formas.get(nome).map(|f| f.rules.len()).unwrap_or(0);
            for indice in 0..qtd_regras {
                let Some(regra) = self.formas.get(nome).and_then(|f| f.rules.get(indice)) else {
                    break;
                };
                let leitura = self.fxp.read_sensor(&regra.sensor, &mut self.caderno);
                let valor_sensor = match leitura {
                    Ok(v) => v,
                    Err(FalhaSensor::NaoRegistrado) | Err(FalhaSensor::Inacessivel) => {
                        // falha de I/O já alertada; condição NÃO avaliada (§4.7)
                        continue;
                    }
                };
                self.caderno.sensor_read(&regra.sensor, valor_sensor);
                if regra.op.avalia(valor_sensor, regra.threshold) {
                    self.caderno.alert(
                        &format!(
                            "Condição de revisão disparada para '{nome}': {} {} {} (lido: {valor_sensor})",
                            regra.sensor,
                            regra.op.simbolo(),
                            fmt_threshold(regra.threshold)
                        ),
                        Json::obj([
                            ("forma", Json::str(nome)),
                            ("sensor", Json::str(&regra.sensor)),
                        ]),
                    );
                    let actions = regra.actions.clone();
                    if self.execute_actions(nome, &actions) {
                        disparou = true;
                        let restantes = qtd_regras - indice - 1;
                        if restantes > 0 {
                            // review_short_circuit: regras seguintes da mesma
                            // review não são avaliadas naquele tick — sem
                            // revogar atuações já despachadas (§4.2/§4.5)
                            self.caderno.record(
                                kinds::REVIEW_SHORT_CIRCUIT,
                                &format!(
                                    "'{nome}': {restantes} regra(s) seguinte(s) não avaliada(s) neste tick."
                                ),
                                Json::obj([
                                    ("forma", Json::str(nome)),
                                    ("regras_restantes", Json::num(restantes as f64)),
                                ]),
                            );
                        }
                        break;
                    }
                }
            }
            if disparou || !self.formas.contains_key(nome) {
                if self.formas.contains_key(nome) {
                    // prazos pulados neste tick voltam para o próximo (o
                    // estado decide — mesmo contrato do protótipo da Etapa 1;
                    // Etapa 5: forma DISSOLVIDA não re-agenda — nada de lixo
                    // de prazos órfãos no heap do escalonador)
                    for (prazo, versao) in &vencidos_forma {
                        self.scheduler.agendar(nome, *prazo, agora + self.tick_seconds, *versao);
                    }
                    // reclassificação (ordem intacta): próxima forma
                    i += 1;
                }
                // senão: dissolvida/subvertida neste tick — `ordem` encolheu
                // na posição corrente; NÃO avançar o índice
                continue;
            }

            // versões vivas (validade das entradas do escalonador)
            let (eh_neq, tem_regras, m_viva, h_viva) = self
                .formas
                .get(nome)
                .map(|f| {
                    (
                        f.conjugation == Conjugation::Nonequilibrium,
                        !f.rules.is_empty(),
                        f.manutencao_versao,
                        f.horizon_versao,
                    )
                })
                .unwrap_or((false, false, 0, 0));

            // marca de dissolução NESTE tick: `ordem` encolheu na posição
            // corrente → `continue` SEM avançar o índice (o corpo termina em
            // `if dissolveu { continue; } i += 1;`)
            let mut dissolveu = false;

            // 2d. manutenção (apenas nonequilibrium) — keep implícito com
            //     regra ativa; colapso no primeiro vencimento estrito sem regra
            if eh_neq {
                if tem_regras {
                    // manutenção implícita (FORMAL §4.1 ii)
                    let (versao, prazo) = {
                        let f = self.formas.get_mut(nome).unwrap();
                        f.keep(agora);
                        f.manutencao_versao += 1;
                        (
                            f.manutencao_versao,
                            f.manutencao.as_ref().map(|m| m.ultima + m.deadline_s).unwrap_or(agora),
                        )
                    };
                    self.scheduler.agendar(nome, Prazo::Manutencao, prazo, versao);
                } else {
                    let venceu_prazo = vencidos_forma.iter().any(|(p, _)| *p == Prazo::Manutencao);
                    let vencida_agora = self
                        .formas
                        .get(nome)
                        .map(|f| f.manutencao_vencida(agora))
                        .unwrap_or(false);
                    if venceu_prazo && vencida_agora {
                        let prazo_s = self
                            .formas
                            .get(nome)
                            .and_then(|f| f.manutencao.as_ref())
                            .map(|m| m.deadline_s)
                            .unwrap_or(0.0);
                        self.caderno.colapso(
                            &format!(
                                "Prazo de manutenção de '{nome}' expirou! (sem keep() por {prazo_s}s)"
                            ),
                            Json::obj([("forma", Json::str(nome))]),
                        );
                        self.dissolve_form(nome, kinds::COLLAPSE_MAINTENANCE);
                        dissolveu = true;
                    }
                    if !dissolveu {
                        // limite exato ainda sustenta: reagenda o prazo atual
                        for (prazo, versao) in &vencidos_forma {
                            if *prazo == Prazo::Manutencao && *versao == m_viva {
                                self.scheduler
                                    .agendar(nome, *prazo, agora + self.tick_seconds, *versao);
                            }
                        }
                    }
                }
            }

            // 2e. horizon — apenas se a forma seguir ativa (FORMAL §4.2)
            if !dissolveu {
                let venceu_horizon = vencidos_forma.iter().any(|(p, _)| *p == Prazo::Horizon);
                let esgotado_agora = self
                    .formas
                    .get(nome)
                    .map(|f| f.horizon_esgotado(agora))
                    .unwrap_or(false);
                if venceu_horizon && esgotado_agora {
                    self.caderno.warn(
                        &format!("Horizonte de validade de '{nome}' esgotou-se. Dissolvendo."),
                        Json::obj([("forma", Json::str(nome))]),
                    );
                    self.dissolve_form(nome, kinds::DISSOLVE_HORIZON);
                    dissolveu = true;
                } else if venceu_horizon {
                    // borda de arredondamento: reagenda para o próximo tick
                    for (prazo, versao) in &vencidos_forma {
                        if *prazo == Prazo::Horizon && *versao == h_viva {
                            self.scheduler
                                .agendar(nome, *prazo, agora + self.tick_seconds, *versao);
                        }
                    }
                }
            }

            if dissolveu {
                continue; // `ordem` encolheu na posição corrente — sem avançar
            }
            i += 1;
        }
    }

    // ------------------------------------------------------------------
    // Ações (FORMAL §4.2/§4.5/§4.6)
    // ------------------------------------------------------------------
    /// Executa a action_list na ordem declarada. Devolve true se a forma
    /// deixou de existir na conjugação anterior (dissolvida/reclassificada).
    fn execute_actions(&mut self, nome: &str, actions: &[ActionRt]) -> bool {
        if !self.formas.contains_key(nome) {
            // ação de revisão sobre forma já dissolvida no mesmo tick é
            // ignorada, com registro (FORMAL §4.1)
            self.caderno.record(
                kinds::REVIEW_AFTER_DISSOLUTION,
                &format!("Ação de revisão sobre '{nome}' ignorada: forma já dissolvida neste tick."),
                Json::obj([("forma", Json::str(nome))]),
            );
            return true;
        }
        let mut doomed = false;
        for action in actions {
            match action {
                ActionRt::Dissolve => {
                    self.dissolve_form(nome, kinds::DISSOLVE_RULE);
                    return true;
                }
                ActionRt::Subvert => {
                    // §4.5: subvert não cancela as ações seguintes da mesma
                    // regra — em particular, qualquer act associado é enviado
                    self.subvert_form(nome);
                    doomed = true;
                }
                ActionRt::ReclassifyEquilibrium => {
                    self.reclassify_equilibrium(nome);
                    return true;
                }
                ActionRt::ReclassifyNonequilibrium => {
                    if self.reclassify_nonequilibrium(nome) {
                        return true;
                    }
                }
                ActionRt::NotifyShutdown => {
                    // §4.6: não dissolve, não interrompe as ações seguintes
                    self.caderno.warn(
                        &format!(
                            "Interrupção do sistema! Desligando cargas secundárias ligadas a '{nome}'."
                        ),
                        Json::obj([("forma", Json::str(nome))]),
                    );
                }
                ActionRt::Act { ator, valor } => {
                    // Etapa 3 (FORMAL §4.5): act na mesma regra após subvert
                    // entra na fila do FXP com prioridade máxima.
                    let prioridade = if doomed { PRIORIDADE_SUBVERT } else { PRIORIDADE_NORMAL };
                    let outcome = self
                        .fxp
                        .act_with_priority(ator, valor.clone(), prioridade, &mut self.caderno);
                    if !outcome.ok() {
                        self.caderno.alert(
                            &format!("Falha na atuação do ator '{ator}' para a forma '{nome}'."),
                            Json::obj([
                                ("forma", Json::str(nome)),
                                ("ator", Json::str(ator)),
                                ("outcome", Json::str(format!("{outcome:?}"))),
                            ]),
                        );
                    }
                }
            }
        }
        if doomed {
            // dissolução da forma subvertida dentro do mesmo tick (§4.5)
            self.dissolve_form(nome, kinds::DISSOLVE_SUBVERT);
            return true;
        }
        false
    }

    /// `event→equilibrium` e `nonequilibrium→equilibrium` (FORMAL §4.1):
    /// persiste em disco (`.vl` canônico + SHA-256) e converte.
    /// `equilibrium→equilibrium` não é transição da matriz — no-op auditado.
    fn reclassify_equilibrium(&mut self, nome: &str) {
        let form = match self.formas.get(nome) {
            Some(f) => f.clone(),
            None => return,
        };
        if form.conjugation == Conjugation::Equilibrium {
            self.caderno.warn(
                &format!(
                    "reclassify_as_equilibrium sobre '{nome}' (já equilibrium) — sem efeito (matriz de transições, FORMAL §4.1)."
                ),
                Json::obj([("forma", Json::str(nome)), ("de", Json::str("equilibrium"))]),
            );
            return;
        }
        // horizon ABSOLUTO: creation_time original é preservado (§4.1)
        let mut nova = form.clone();
        nova.conjugation = Conjugation::Equilibrium;
        nova.currency = Conjugation::Equilibrium.currency_padrao().into();
        nova.manutencao = None;
        nova.manutencao_versao = 0;
        nova.cost_bytes = None; // tamanho real gravado (FORMAL §4.1)
        nova.horizon_versao += 1;

        self.caderno.record(
            kinds::TRANSICAO,
            &format!("Forma '{nome}' reclassificada para 'equilibrium' (persistida)."),
            Json::obj([
                ("forma", Json::str(nome)),
                ("de", Json::str(form.conjugation.nome())),
                ("para", Json::str("equilibrium")),
            ]),
        );
        self.bind(nova.clone());
        if let Err(e) = self.persistir(&nova) {
            self.caderno.alert(
                &format!("Falha ao persistir '{nome}': {e}"),
                Json::obj([
                    ("forma", Json::str(nome)),
                    ("motivo", Json::str("persistencia_falhou")),
                ]),
            );
        }
    }

    /// `equilibrium→nonequilibrium` e `nonequilibrium→nonequilibrium` (keep).
    /// Sem deadline declarado: erro de runtime registrado — a forma permanece
    /// (FORMAL §3).
    fn reclassify_nonequilibrium(&mut self, nome: &str) -> bool {
        let form = match self.formas.get(nome) {
            Some(f) => f.clone(),
            None => return false,
        };
        let Some(deadline) = form.declared_maintenance_deadline else {
            self.caderno.record(
                kinds::RECLASSIFY_SEM_DEADLINE,
                &format!(
                    "reclassify_as_nonequilibrium recusado para '{nome}': sem maintenance_deadline declarado (FORMAL §3). A forma permanece como estava."
                ),
                Json::obj([("forma", Json::str(nome))]),
            );
            return true; // a conjugação "tentou mudar" → short circuit da review
        };
        let modo = form.exchange_mode.clone().unwrap_or_else(|| "cooperation".into());
        let mut nova = form.clone();
        nova.conjugation = Conjugation::Nonequilibrium;
        nova.currency = Conjugation::Nonequilibrium.currency_padrao().into();
        nova.manutencao = Some(Manutencao {
            deadline_s: deadline,
            // semântica do protótipo: última manutenção parte da criação
            // original (horizon absoluto; keep implícito/regra renova em t+1)
            ultima: form.creation_time,
        });
        nova.manutencao_versao += 1;
        nova.horizon_versao += 1;
        nova.exchange_mode = Some(modo);
        self.caderno.record(
            kinds::TRANSICAO,
            &format!("Forma '{nome}' reclassificada para 'nonequilibrium' (trabalho ativo)."),
            Json::obj([
                ("forma", Json::str(nome)),
                ("de", Json::str(form.conjugation.nome())),
                ("para", Json::str("nonequilibrium")),
            ]),
        );
        self.bind(nova);
        true
    }

    // ------------------------------------------------------------------
    // Persistência (FORMAL §4.1): `.vl` canônico + SHA-256 no Caderno
    // ------------------------------------------------------------------
    fn persistir(&mut self, form: &Form) -> Result<(String, String), String> {
        std::fs::create_dir_all(&self.persistence_dir)
            .map_err(|e| format!("diretório {}: {e}", self.persistence_dir.display()))?;
        let decl = form_para_ast(form);
        let conteudo = vbl_lang::canon::form_to_vl(&decl);
        let dados = conteudo.as_bytes();
        let caminho = self.persistence_dir.join(format!("{}.vl", form.name));
        std::fs::write(&caminho, dados).map_err(|e| format!("{}: {e}", caminho.display()))?;
        let sha256 = crate::notebook::sha256_hex(dados);
        let bytes = dados.len() as u64;
        self.caderno.record(
            kinds::PERSISTENCIA,
            &format!("Forma '{}' persistida como `.vl` canônico.", form.name),
            Json::obj([
                ("forma", Json::str(&form.name)),
                ("caminho", Json::str(caminho.display().to_string())),
                ("sha256", Json::str(&sha256)),
                ("bytes", Json::num(bytes as f64)),
            ]),
        );
        // cost_bytes ausente passa a valer o tamanho real gravado (FORMAL §4.1)
        if let Some(f) = self.formas.get_mut(&form.name) {
            if f.cost_bytes.is_none() {
                f.cost_bytes = Some(bytes);
            }
        }
        // sidecar: creation_time para recarregar com horizon absoluto íntegro
        let _ = crate::persist::gravar_sidecar(
            &self.persistence_dir,
            &form.name,
            form.creation_time,
        );
        self.fxp.add_disk_bytes(1024); // escrita simulada no suporte estável
        Ok((caminho.display().to_string(), sha256))
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
pub fn form_para_ast(form: &Form) -> vbl_lang::FormDecl {
    use vbl_lang::FormAttrs;
    let span = vbl_lang::Span::default();
    let value = match &form.value {
        Value::Num(n) => vbl_lang::Expression::num(*n, span),
        Value::Str(s) => vbl_lang::Expression::str(s.clone(), span),
        Value::Ident(s) => vbl_lang::Expression::ident(s.clone(), span),
    };
    let horizon = duracao_ast(form.horizon_s, span);
    let maintenance_deadline = form.manutencao.as_ref().map(|m| duracao_ast(m.deadline_s, span));
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
        currency: if form.currency == form.conjugation.currency_padrao() {
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
fn duracao_ast(segundos: f64, span: vbl_lang::Span) -> vbl_lang::Duration {
    use vbl_lang::TimeUnit;
    let (valor, unit) = if segundos.fract() == 0.0 {
        (segundos, TimeUnit::S)
    } else if (segundos * 1e3).fract().abs() < 1e-9 {
        (segundos * 1e3, TimeUnit::Ms)
    } else {
        (segundos, TimeUnit::S)
    };
    vbl_lang::Duration { valor, unit, span }
}

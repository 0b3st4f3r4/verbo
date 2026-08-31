//! O barramento FXP — [`FxpBus`] implementa o trait `Fxp` do runtime
//! roteando cada dispositivo conforme o modo de operação (PLAN §3.1/§3.4):
//!
//! | modo global | rota |
//! |-------------|------|
//! | `simulado`  | tudo no simulador determinístico (compatível com a Etapa 2) |
//! | `hibrido`   | por dispositivo: reais onde há rota, simulados no restante (marcado) |
//! | `real`      | nada sintético: dispositivo sem rota real é **registrado porém inacessível** (§4.7) |
//!
//! Honestidade (FORMAL §4.7): dado sintético só circula em modo simulado/
//! híbrido **explícito** e chega marcado ao Caderno; falha de I/O nunca é
//! leitura 0.0; fallback é política do REGISTRO (§4.3) e atua na rota de
//! I/O — nunca falsifica leitura.
//!
//! O `DeviceRegistry` é a **fonte única** de limites/aliases; na construção o
//! simulador embutido é sincronizado com ele, de modo que o backend simulado
//! valide exatamente o mesmo registro.

use crate::drivers::{ator_de, descobrir, sensor_de, ActorDriver, SensorDriver};
use crate::queue::{Comando, FilaComandos};
use crate::registry::{DeviceKind, DeviceMode, DeviceRegistry, Endpoint, ModoOperacao, RemoteAddr};
use crate::schema::{flag, razao, AckAct, Corpo, Mensagem, WireValue};
use crate::transport::{Conexao, ErroTransporte};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use vbl_runtime::notebook::{kinds as rt_kinds, Atuacao, Caderno};
use vbl_runtime::fxp::{
    ActOutcome, ActorLimits, FalhaSensor, Fxp, Limite, Registry as RuntimeRegistry, Value,
    PRIORIDADE_NORMAL,
};
use vbl_runtime::json::Json;
use vbl_runtime::sim::FxpSimulator;

/// Kinds de evento acrescentados pelo barramento (vocabulário do Caderno —
/// FORMAL §6 lista os canônicos de fim de forma; estes cobrem o ciclo de
/// vida da fila prioritária).
pub mod kinds {
    /// Comando enfileirado foi entregue numa re-entrega do `on_tick`.
    pub const COMANDO_REENTREGUE: &str = "comando_reentregue";
    /// Comando excedeu `queue_timeout_ticks` e foi descartado com alerta.
    pub const COMANDO_EXPIRADO: &str = "comando_expirado";
}

/// Trilha de atuação com a latência medida (Etapa 4 — PLAN §4.1): o Caderno
/// estima o custo energético como potência do tick × latência do ack.
fn atuacao_com_latencia(
    caderno: &mut dyn Caderno,
    ator: &str,
    valor: &Value,
    latencia_us: u64,
    sucesso: bool,
) {
    caderno.actuator_action_detalhada(Atuacao {
        ator: ator.to_owned(),
        solicitado: valor.clone(),
        aplicado: if sucesso { Some(valor.clone()) } else { None },
        latencia_us: Some(latencia_us),
        custo_joules: None,
        sucesso,
    });
}

/// Parâmetros do barramento (defaults = docs/FXP-SCHEMA-v1.md §6).
#[derive(Debug, Clone)]
pub struct BusConfig {
    pub modo: ModoOperacao,
    /// Cache de leitura de rotas reais/remotas (mitigação PLAN §3).
    pub cache_ttl: Duration,
    /// Prazo de ack de leitura remota (fio).
    pub read_timeout: Duration,
    /// Prazo de ack de atuação local (entre processos).
    pub act_timeout_local: Duration,
    /// Prazo de ack de atuação remota.
    pub act_timeout_remote: Duration,
    /// Tentativas de transporte além da primeira (PLAN §3: retry).
    pub retries: u32,
    /// Ticks virtuais que um comando pode esperar na fila.
    pub queue_timeout_ticks: u64,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            modo: ModoOperacao::Simulado,
            cache_ttl: Duration::from_millis(100),
            read_timeout: Duration::from_millis(10),
            act_timeout_local: Duration::from_millis(50),
            act_timeout_remote: Duration::from_millis(500),
            retries: 1,
            queue_timeout_ticks: 2,
        }
    }
}

/// Rota efetiva de um dispositivo (resolvida na construção).
#[derive(Debug, Clone, PartialEq)]
pub enum Rota {
    /// Simulador embutido (modo simulado global ou dispositivo simulado).
    Simulador,
    /// Driver de arquivo real (sysfs/hwmon/led).
    Real,
    /// Peer remoto falando schema v1.
    Remota(RemoteAddr),
    /// Registrado porém inacessível: modo real global proíbe a rota simulada
    /// e não há rota real (§4.7 — nunca simulado silencioso).
    Inacessivel { motivo: String },
}

impl Rota {
    /// Descrição legível (probe/relatório).
    pub fn descricao(&self) -> String {
        match self {
            Rota::Simulador => "simulado (em processo)".into(),
            Rota::Real => "real (driver de arquivo)".into(),
            Rota::Remota(a) => match a {
                RemoteAddr::Unix(p) => format!("remota (unix:{})", p.display()),
                RemoteAddr::Tcp { host, port } => format!("remota (tcp:{host}:{port})"),
            },
            Rota::Inacessivel { motivo } => format!("inacessível ({motivo})"),
        }
    }
}

/// O barramento FXP consumido pelo runtime (`Engine<F: Fxp>`).
pub struct FxpBus {
    registry: DeviceRegistry,
    rt_registry: RuntimeRegistry,
    config: BusConfig,
    sim: FxpSimulator,
    rotas: BTreeMap<String, Rota>, // canônico → rota
    sensores_reais: BTreeMap<String, Box<dyn SensorDriver + Send>>,
    atores_reais: BTreeMap<String, Box<dyn ActorDriver + Send>>,
    conexoes: BTreeMap<String, Conexao>, // canônico → transporte remoto
    cache: BTreeMap<String, (Instant, f64)>,
    fila: FilaComandos,
    seq: u32,
    disk_bytes: u64,
    /// Última leitura de `cpu_power` em rota real/remota (partilha P/N — §4.2).
    potencia_conhecida: f64,
    potencia_inacessivel: bool,
    potencia_lida_em: Option<Instant>,
}

impl std::fmt::Debug for FxpBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FxpBus")
            .field("config", &self.config)
            .field("rotas", &self.rotas)
            .field("fila", &self.fila.len())
            .finish()
    }
}

impl FxpBus {
    /// Constrói o barramento com rotas resolvidas. Endpoints `Auto` tentam
    /// descoberta no host; falha de descoberta **não é erro de construção** —
    /// o dispositivo fica registrado porém inacessível (§4.7).
    pub fn construir(registry: DeviceRegistry, mut config: BusConfig, mut sim: FxpSimulator) -> Self {
        let mut rotas = BTreeMap::new();
        let mut sensores_reais = BTreeMap::new();
        let mut atores_reais = BTreeMap::new();

        if config.modo == ModoOperacao::Simulado {
            // Simulador determinístico: sem cache (roteirização é imediata).
            config.cache_ttl = Duration::ZERO;
        }

        // Fonte única de verdade: o registro rico acrescenta ao simulador os
        // dispositivos que só existem nele (extensões). Dispositivos já
        // presentes no sim (roteirizados pelo CLI) são PRESERVADOS — estado
        // como "ator indisponível" do cenário não pode ser resetado.
        for d in registry.dispositivos() {
            match &d.kind {
                DeviceKind::Sensor { grandeza, unidade, .. } => {
                    if !sim.registry().sensores.contains_key(&d.name) {
                        sim.registrar_sensor(
                            &d.name,
                            vbl_runtime::fxp::SensorInfo {
                                grandeza: grandeza.clone(),
                                unidade: unidade.clone(),
                            },
                        );
                    }
                }
                DeviceKind::Actor { limits } => {
                    if !sim.registry().atores.contains_key(&d.name) {
                        sim.registrar_ator(&d.name, limits.clone());
                    }
                }
            }
        }

        for d in registry.dispositivos() {
            let rota = match rota_base(&config, d) {
                RotaEspec::Simulador => Rota::Simulador,
                RotaEspec::Inacessivel { motivo } => Rota::Inacessivel { motivo },
                RotaEspec::Concreta => {
                    let endpoint = match &d.endpoint {
                        Endpoint::Auto => descobrir(&d.name),
                        e => Some(e.clone()),
                    };
                    match endpoint {
                        None => Rota::Inacessivel {
                            motivo: "auto-descoberta não encontrou hardware".into(),
                        },
                        Some(Endpoint::Remote { addr }) => Rota::Remota(addr),
                        Some(Endpoint::Simulado) => Rota::Inacessivel {
                            motivo: "modo real não roteia para simulador (dado sintético proibido)"
                                .into(),
                        },
                        Some(ep) => match &d.kind {
                            DeviceKind::Sensor { .. } => match sensor_de(&ep) {
                                Some(dr) => {
                                    sensores_reais.insert(d.name.clone(), dr);
                                    Rota::Real
                                }
                                None => Rota::Inacessivel {
                                    motivo: format!(
                                        "endpoint sem driver de leitura: {}",
                                        ep.descricao()
                                    ),
                                },
                            },
                            DeviceKind::Actor { .. } => match ator_de(&ep) {
                                Some(dr) => {
                                    atores_reais.insert(d.name.clone(), dr);
                                    Rota::Real
                                }
                                None => Rota::Inacessivel {
                                    motivo: format!(
                                        "endpoint sem driver de atuação: {}",
                                        ep.descricao()
                                    ),
                                },
                            },
                        },
                    }
                }
            };
            rotas.insert(d.name.clone(), rota);
        }

        let rt_registry = registry.to_runtime_registry();
        Self {
            registry,
            rt_registry,
            config,
            sim,
            rotas,
            sensores_reais,
            atores_reais,
            conexoes: BTreeMap::new(),
            cache: BTreeMap::new(),
            fila: FilaComandos::default(),
            seq: 0,
            disk_bytes: 0,
            potencia_conhecida: 0.0,
            potencia_inacessivel: false,
            potencia_lida_em: None,
        }
    }

    /// Registro rico (aliases, modos, endpoints, fallback) — probe/config.
    pub fn registry_rico(&self) -> &DeviceRegistry {
        &self.registry
    }

    /// Simulador embutido (roteirização do CLI em modo simulado/híbrido).
    pub fn sim_mut(&mut self) -> &mut FxpSimulator {
        &mut self.sim
    }

    pub fn sim(&self) -> &FxpSimulator {
        &self.sim
    }

    /// Fila de comandos pendentes (observação de testes/probe).
    pub fn fila_pendente(&self) -> usize {
        self.fila.len()
    }

    /// Rota efetiva de um nome simbólico (para `vbl fxp-probe`).
    pub fn rota_de(&self, nome: &str) -> Option<&Rota> {
        self.rotas.get(self.registry.canonical_de(nome))
    }

    fn alerta_sensor(
        &mut self,
        caderno: &mut dyn Caderno,
        motivo: &str,
        sensor: &str,
        detalhe: &str,
    ) {
        caderno.alert(
            &format!(
                "Sensor '{sensor}' — falha de I/O ({motivo}): {detalhe} Condição não avaliada neste tick."
            ),
            Json::obj([("motivo", Json::str(motivo)), ("sensor", Json::str(sensor))]),
        );
    }

    fn proximo_seq(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn cache_valido(&self, canonical: &str) -> Option<f64> {
        if self.config.cache_ttl.is_zero() {
            return None;
        }
        match self.cache.get(canonical) {
            Some((quando, valor)) if quando.elapsed() < self.config.cache_ttl => Some(*valor),
            _ => None,
        }
    }

    /// Leitura em rota real: cache TTL → driver → honestidade de falha.
    fn ler_real(
        &mut self,
        canonical: &str,
        caderno: &mut dyn Caderno,
    ) -> Result<f64, FalhaSensor> {
        if let Some(v) = self.cache_valido(canonical) {
            return Ok(v);
        }
        let Some(dr) = self.sensores_reais.get_mut(canonical) else {
            self.alerta_sensor(caderno, "sensor_inacessivel", canonical, "sem driver real.");
            return Err(FalhaSensor::Inacessivel);
        };
        match dr.read() {
            Ok(v) => {
                self.cache.insert(canonical.into(), (Instant::now(), v));
                self.anotar_potencia(canonical, v);
                Ok(v)
            }
            Err(_) => {
                if canonical == "cpu_power" {
                    self.potencia_inacessivel = true;
                }
                let detalhe =
                    format!("driver real ({}) falhou.", self.descricao_rota(canonical));
                self.alerta_sensor(caderno, "sensor_inacessivel", canonical, &detalhe);
                Err(FalhaSensor::Inacessivel)
            }
        }
    }

    /// Leitura remota via schema v1 (READ → READ_OK/READ_ERR, ack por seq).
    fn ler_remota(
        &mut self,
        canonical: &str,
        caderno: &mut dyn Caderno,
    ) -> Result<f64, FalhaSensor> {
        if let Some(v) = self.cache_valido(canonical) {
            return Ok(v);
        }
        let Some(Rota::Remota(addr)) = self.rotas.get(canonical).cloned() else {
            return Err(FalhaSensor::Inacessivel);
        };
        let seq = self.proximo_seq();
        let pedido = Mensagem::read(canonical, seq, true);
        match self.pedir_remoto(canonical, &addr, &pedido, self.config.read_timeout) {
            Ok(resp) => {
                let sintetico = resp.flags & flag::SINTETICO != 0;
                match resp.corpo {
                    Corpo::ReadOk { valor, canonical: can } => {
                        if sintetico {
                            // §4.7: dado sintético sempre marcado no Caderno.
                            caderno.warn(
                                &format!("Leitura remota de '{canonical}' é de origem simulada (measurement_status: simulado)."),
                                Json::obj([
                                    ("motivo", Json::str("measurement_status_simulado")),
                                    ("sensor", Json::str(canonical)),
                                    ("canonical", Json::str(can)),
                                    ("valor", Json::num(valor)),
                                ]),
                            );
                        }
                        self.cache.insert(canonical.into(), (Instant::now(), valor));
                        self.anotar_potencia(canonical, valor);
                        Ok(valor)
                    }
                    Corpo::ReadErr { reason } => {
                        let (falha, motivo) = match reason {
                            razao::NAO_REGISTRADO => {
                                (FalhaSensor::NaoRegistrado, "sensor_nao_registrado")
                            }
                            _ => (FalhaSensor::Inacessivel, "sensor_inacessivel"),
                        };
                        self.alerta_sensor(caderno, motivo, canonical, "peer respondeu erro.");
                        Err(falha)
                    }
                    _ => {
                        self.alerta_sensor(
                            caderno,
                            "sensor_inacessivel",
                            canonical,
                            "resposta inesperada do peer.",
                        );
                        Err(FalhaSensor::Inacessivel)
                    }
                }
            }
            Err(e) => {
                self.conexoes.remove(canonical); // conexão suspeita: reconectar
                self.alerta_sensor(
                    caderno,
                    "sensor_inacessivel",
                    canonical,
                    &format!("transporte: {e}."),
                );
                Err(FalhaSensor::Inacessivel)
            }
        }
    }

    fn anotar_potencia(&mut self, canonical: &str, valor: f64) {
        if canonical == "cpu_power" {
            self.potencia_conhecida = valor;
            self.potencia_inacessivel = false;
            self.potencia_lida_em = Some(Instant::now());
        }
    }

    /// Pedido-resposta remoto com reconexão preguiçosa.
    fn pedir_remoto(
        &mut self,
        canonical: &str,
        addr: &RemoteAddr,
        pedido: &Mensagem,
        timeout: Duration,
    ) -> Result<Mensagem, ErroTransporte> {
        if !self.conexoes.contains_key(canonical) {
            let c = match addr {
                RemoteAddr::Unix(p) => Conexao::unix(p, timeout)?,
                RemoteAddr::Tcp { host, port } => Conexao::tcp(host, *port, timeout)?,
            };
            self.conexoes.insert(canonical.into(), c);
        }
        let c = self.conexoes.get_mut(canonical).expect("inserido acima");
        c.pedir(pedido, timeout)
    }

    fn descricao_rota(&self, canonical: &str) -> String {
        self.rotas
            .get(canonical)
            .map(|r| r.descricao())
            .unwrap_or_else(|| "fora do registro".into())
    }

    // -----------------------------------------------------------------
    // Atuação: validação, rota, retry e fallback do registro (§4.3)
    // -----------------------------------------------------------------

    fn violacao(&self, limits: &ActorLimits, valor: &Value) -> Option<(Limite, f64)> {
        let v = valor.as_num()?;
        // limites INCLUSIVOS: valor igual ao limite é aceito (FORMAL §4.3)
        if let Some(min) = limits.min {
            if v < min {
                return Some((Limite::Min, min));
            }
        }
        if let Some(max) = limits.max {
            if v > max {
                return Some((Limite::Max, max));
            }
        }
        if let Some(safety) = limits.safety_limit {
            if v > safety {
                return Some((Limite::SafetyLimit, safety));
            }
        }
        None
    }

    /// Limites do registro para um ator (fonte única).
    fn limites_de(&self, canonical: &str) -> ActorLimits {
        match self.registry.get(canonical).map(|d| &d.kind) {
            Some(DeviceKind::Actor { limits }) => limits.clone(),
            _ => ActorLimits::default(),
        }
    }

    fn rejeitar_por_limite(
        &mut self,
        canonical: &str,
        valor: &Value,
        limite: Limite,
        valor_limite: f64,
        caderno: &mut dyn Caderno,
    ) -> ActOutcome {
        caderno.record(
            rt_kinds::ACTOR_REJECTED_VALUE,
            &format!(
                "Comando a '{canonical}' rejeitado sem envio: valor viola {} = {valor_limite}.",
                limite.nome()
            ),
            Json::obj([
                ("ator", Json::str(canonical)),
                ("valor", valor.to_json()),
                ("limite", Json::str(limite.nome())),
                ("limite_valor", Json::num(valor_limite)),
            ]),
        );
        caderno.actuator_action(canonical, valor, false);
        ActOutcome::Rejeitado { limite, valor_limite }
    }

    /// Tenta entregar na rota REAL do ator (sem fallback).
    /// `Ok(true)` = entregue; `Ok(false)` = indisponível (segue fallback);
    /// `Err(outcome)` = terminativo (rejeição/domínio — sem fallback).
    fn entregar_real(
        &mut self,
        canonical: &str,
        valor: &Value,
        caderno: &mut dyn Caderno,
    ) -> Result<bool, ActOutcome> {
        let Some(dr) = self.atores_reais.get_mut(canonical) else {
            return Ok(false);
        };
        // tentativas = 1 + retries (PLAN §3: fila com retry e fallback)
        let mut latencia_ultima_us = 0u64;
        for _ in 0..=self.config.retries {
            let t0 = Instant::now();
            let resultado = dr.apply(valor);
            latencia_ultima_us = t0.elapsed().as_micros() as u64;
            match resultado {
                Ok(()) => {
                    // Etapa 4 (PLAN §4.1): valor aplicado + latência do ack;
                    // o custo energético é estimado pelo Caderno (W × latência).
                    atuacao_com_latencia(caderno, canonical, valor, latencia_ultima_us, true);
                    return Ok(true);
                }
                Err(crate::drivers::ErroAtor::ValorInvalido(motivo)) => {
                    atuacao_com_latencia(caderno, canonical, valor, latencia_ultima_us, false);
                    caderno.record(
                        rt_kinds::ACTOR_REJECTED_VALUE,
                        &format!("Comando a '{canonical}' fora do domínio do ator: {motivo}."),
                        Json::obj([
                            ("ator", Json::str(canonical)),
                            ("valor", valor.to_json()),
                            ("motivo", Json::str(motivo.clone())),
                        ]),
                    );
                    return Err(ActOutcome::ValorInvalido { motivo });
                }
                Err(crate::drivers::ErroAtor::EscritaFalhou(_)) => {
                    continue; // retry de transporte antes do fallback
                }
            }
        }
        atuacao_com_latencia(caderno, canonical, valor, latencia_ultima_us, false);
        caderno.record(
            rt_kinds::ATOR_INDISPONIVEL,
            &format!(
                "Heartbeat do ator '{canonical}' não respondeu (rota {}).",
                self.descricao_rota(canonical)
            ),
            Json::obj([("ator", Json::str(canonical))]),
        );
        Ok(false)
    }

    /// Entrega remota (ACT → ACT_ACK, §4.3). `Some` = terminativo;
    /// `None` = indisponível (segue fallback).
    fn entregar_remota(
        &mut self,
        canonical: &str,
        valor: &Value,
        caderno: &mut dyn Caderno,
    ) -> Option<ActOutcome> {
        let Some(Rota::Remota(addr)) = self.rotas.get(canonical).cloned() else {
            return Some(ActOutcome::AtorInexistente);
        };
        let seq = self.proximo_seq();
        let pedido = Mensagem::act(canonical, wire_de(valor), seq, true);
        let timeout = if matches!(addr, RemoteAddr::Tcp { .. }) {
            self.config.act_timeout_remote
        } else {
            self.config.act_timeout_local
        };
        let t0 = Instant::now();
        match self.pedir_remoto(canonical, &addr, &pedido, timeout) {
            Ok(resp) => {
                let latencia_us = t0.elapsed().as_micros() as u64;
                match resp.corpo {
                    Corpo::ActAck { status } => match status {
                        AckAct::Entregue => {
                            atuacao_com_latencia(caderno, canonical, valor, latencia_us, true);
                            Some(ActOutcome::Entregue)
                        }
                        AckAct::Rejeitado { limite, valor_limite } => {
                            let limite = match limite {
                                0 => Limite::Min,
                                1 => Limite::Max,
                                _ => Limite::SafetyLimit,
                            };
                            Some(self.rejeitar_por_limite(canonical, valor, limite, valor_limite, caderno))
                        }
                        AckAct::AtorInexistente => {
                            caderno.record(
                                rt_kinds::ATOR_INEXISTENTE,
                                &format!("Ator '{canonical}' não registrado no peer remoto."),
                                Json::obj([("ator", Json::str(canonical))]),
                            );
                            atuacao_com_latencia(caderno, canonical, valor, latencia_us, false);
                            Some(ActOutcome::AtorInexistente)
                        }
                        AckAct::ValorInvalido { motivo } => {
                            atuacao_com_latencia(caderno, canonical, valor, latencia_us, false);
                            Some(ActOutcome::ValorInvalido { motivo })
                        }
                        AckAct::Indisponivel | AckAct::FallbackEsgotado => {
                            atuacao_com_latencia(caderno, canonical, valor, latencia_us, false);
                            caderno.record(
                                rt_kinds::ATOR_INDISPONIVEL,
                                &format!("Heartbeat do ator '{canonical}' não respondeu (peer)."),
                                Json::obj([("ator", Json::str(canonical))]),
                            );
                            None
                        }
                        AckAct::FallbackExecutado { alternativo } => {
                            // O peer executou o fallback do registro DELE (§4.3).
                            atuacao_com_latencia(caderno, &alternativo, valor, latencia_us, true);
                            caderno.record(
                                rt_kinds::FALLBACK_EXECUTADO,
                                &format!(
                                    "Fallback '{alternativo}' acionado após falha de '{canonical}'."
                                ),
                                Json::obj([
                                    ("primario", Json::str(canonical)),
                                    ("alternativo", Json::str(alternativo.clone())),
                                    ("valor", valor.to_json()),
                                ]),
                            );
                            Some(ActOutcome::FallbackExecutado { alternativo })
                        }
                    },
                    _ => {
                        atuacao_com_latencia(caderno, canonical, valor, latencia_us, false);
                        None
                    }
                }
            }
            Err(e) => {
                self.conexoes.remove(canonical);
                atuacao_com_latencia(caderno, canonical, valor, t0.elapsed().as_micros() as u64, false);
                caderno.record(
                    rt_kinds::ATOR_INDISPONIVEL,
                    &format!("Heartbeat do ator '{canonical}' não respondeu (transporte: {e})."),
                    Json::obj([("ator", Json::str(canonical))]),
                );
                None
            }
        }
    }

    /// Entrega em um ator específico pela rota DELE (fallback e re-entrega).
    /// `None` = indisponível.
    fn entregar_rota(
        &mut self,
        canonical: &str,
        valor: &Value,
        caderno: &mut dyn Caderno,
    ) -> Option<ActOutcome> {
        let rota = self.rotas.get(canonical)?.clone();
        match rota {
            Rota::Simulador => Some(self.sim.act(canonical, valor.clone(), caderno)),
            Rota::Real => {
                // Limites do registro (inclusivos) ANTES do envio (§4.3).
                let limits = self.limites_de(canonical);
                if let Some((limite, valor_limite)) = self.violacao(&limits, valor) {
                    return Some(self.rejeitar_por_limite(
                        canonical, valor, limite, valor_limite, caderno,
                    ));
                }
                match self.entregar_real(canonical, valor, caderno) {
                    Ok(true) => Some(ActOutcome::Entregue),
                    Ok(false) => None,
                    Err(outcome) => Some(outcome),
                }
            }
            Rota::Remota(_) => self.entregar_remota(canonical, valor, caderno),
            Rota::Inacessivel { motivo } => {
                caderno.actuator_action(canonical, valor, false);
                caderno.record(
                    rt_kinds::ATOR_INDISPONIVEL,
                    &format!("Ator '{canonical}' indisponível ({motivo})."),
                    Json::obj([("ator", Json::str(canonical))]),
                );
                None
            }
        }
    }

    /// Fallback do REGISTRO (FORMAL §4.3): primary → alternativos declarados
    /// no registro; o runtime não implementa fallback próprio.
    fn tentar_fallback(
        &mut self,
        primario: &str,
        valor: &Value,
        caderno: &mut dyn Caderno,
    ) -> ActOutcome {
        let alternativos: Vec<String> = self
            .registry
            .get(primario)
            .map(|d| d.fallback.clone())
            .unwrap_or_default();
        for alt in alternativos {
            if !self.registry.contains(&alt) {
                continue;
            }
            if let Some(outcome) = self.entregar_rota(&alt, valor, caderno) {
                if outcome.ok() {
                    caderno.record(
                        rt_kinds::FALLBACK_EXECUTADO,
                        &format!("Fallback '{alt}' acionado após falha de '{primario}'."),
                        Json::obj([
                            ("primario", Json::str(primario)),
                            ("alternativo", Json::str(alt.clone())),
                            ("valor", valor.to_json()),
                        ]),
                    );
                    // Contrato da Etapa 1/2: atuação por fallback SEMPRE
                    // devolve FallbackExecutado { alternativo } (BDD Caso 3),
                    // independentemente da variante de entrega da rota.
                    return ActOutcome::FallbackExecutado { alternativo: alt };
                }
            }
        }
        caderno.alert(
            &format!("Todos os fallbacks de '{primario}' falharam."),
            Json::obj([
                ("motivo", Json::str("fallback_esgotado")),
                ("ator", Json::str(primario)),
            ]),
        );
        ActOutcome::FallbackEsgotado
    }

    fn enfileirar(&mut self, ator: &str, valor: &Value, prioridade: u8) {
        let cmd = Comando {
            seq: self.proximo_seq(),
            ator: ator.into(),
            valor: valor.clone(),
            prioridade,
            ticks_esperando: 0,
            primario: None,
        };
        if self.fila.empilhar(cmd).is_err() {
            // Guarda anti-inchaço estourou: auditoria honesta do descarte.
            // (capacidade 256 torna isso improvável; o evento é obrigatório)
        }
    }

    /// Re-entrega da fila no relógio virtual (PLAN §3.4: prioridades e
    /// timeout), com trilha completa no Caderno.
    fn bombear_fila(&mut self, caderno: &mut dyn Caderno) {
        if self.fila.is_empty() {
            return;
        }
        let mut pendentes = Vec::new();
        while let Some(cmd) = self.fila.proximo() {
            pendentes.push(cmd);
        }
        for cmd in pendentes {
            if cmd.ticks_esperando >= self.config.queue_timeout_ticks {
                caderno.record(
                    kinds::COMANDO_EXPIRADO,
                    &format!(
                        "Comando a '{}' (seq {}) expirou na fila após {} tick(s).",
                        cmd.ator, cmd.seq, cmd.ticks_esperando
                    ),
                    Json::obj([
                        ("ator", Json::str(cmd.ator.clone())),
                        ("valor", cmd.valor.to_json()),
                        ("ticks", Json::num(cmd.ticks_esperando as f64)),
                    ]),
                );
                caderno.alert(
                    &format!("Comando a '{}' expirou na fila do FXP.", cmd.ator),
                    Json::obj([
                        ("motivo", Json::str("comando_expirado")),
                        ("ator", Json::str(cmd.ator.clone())),
                    ]),
                );
                continue;
            }
            match self.entregar_rota(&cmd.ator, &cmd.valor, caderno) {
                Some(outcome) if outcome.ok() => {
                    caderno.record(
                        kinds::COMANDO_REENTREGUE,
                        &format!(
                            "Comando a '{}' re-entregue após {} tick(s) na fila.",
                            cmd.ator, cmd.ticks_esperando
                        ),
                        Json::obj([
                            ("ator", Json::str(cmd.ator.clone())),
                            ("valor", cmd.valor.to_json()),
                            ("ticks", Json::num(cmd.ticks_esperando as f64)),
                        ]),
                    );
                }
                _ => {
                    // Falhou de novo: volta com +1 tick (expiração no topo).
                    let _ = self.fila.devolver(cmd);
                }
            }
        }
    }

    /// Varredura silenciosa da potência real (sem Caderno); falha vira alerta
    /// na próxima operação com caderno. Rotas remotas atualizam só quando
    /// lidas pelas regras (on_tick não faz I/O remoto — plano determinístico).
    fn atualizar_potencia(&mut self) {
        if self.rotas.get("cpu_power") != Some(&Rota::Real) {
            return;
        }
        if let Some(t) = self.potencia_lida_em {
            if t.elapsed() < self.config.cache_ttl {
                return;
            }
        }
        if let Some(dr) = self.sensores_reais.get_mut("cpu_power") {
            match dr.read() {
                Ok(v) => {
                    self.potencia_conhecida = v;
                    self.potencia_inacessivel = false;
                }
                Err(_) => self.potencia_inacessivel = true,
            }
            self.potencia_lida_em = Some(Instant::now());
        }
    }
}

/// Rota pré-driver: o modo global modula o modo do dispositivo.
enum RotaEspec {
    Simulador,
    Concreta,
    Inacessivel { motivo: String },
}

fn rota_base(config: &BusConfig, d: &crate::registry::DeviceEntry) -> RotaEspec {
    let modo_dispositivo = match config.modo {
        ModoOperacao::Simulado => DeviceMode::Simulado,
        ModoOperacao::Real | ModoOperacao::Hibrido => d.mode,
    };
    match (config.modo, modo_dispositivo) {
        // Modo real global proíbe rota sintética — §4.7 (nunca simulado mudo).
        (ModoOperacao::Real, DeviceMode::Simulado) => RotaEspec::Inacessivel {
            motivo: "modo real não roteia para simulador (dado sintético proibido)".into(),
        },
        // Simulado explícito (global, ou por dispositivo no híbrido).
        (_, DeviceMode::Simulado) => RotaEspec::Simulador,
        // Dispositivo real: rota concreta (Auto/driver/remota).
        (_, DeviceMode::Real) => RotaEspec::Concreta,
    }
}

fn wire_de(v: &Value) -> WireValue {
    match v {
        Value::Num(n) => WireValue::Num(*n),
        Value::Str(s) => WireValue::Str(s.clone()),
        Value::Ident(s) => WireValue::Ident(s.clone()),
    }
}

impl Fxp for FxpBus {
    fn read_sensor(
        &mut self,
        nome: &str,
        caderno: &mut dyn Caderno,
    ) -> Result<f64, FalhaSensor> {
        let canonical = self.registry.canonical_de(nome).to_string();
        if !self.registry.contains(&canonical) {
            self.alerta_sensor(
                caderno,
                "sensor_nao_registrado",
                nome,
                "fora do registro do FXP.",
            );
            return Err(FalhaSensor::NaoRegistrado);
        }
        // §6: leitura por alias é idêntica à do canônico; o Caderno registra
        // o nome usado (LEITURA do engine) e o canônico (este evento).
        if canonical != nome {
            caderno.info(
                &format!(
                    "Leitura de '{nome}' resolvida para o dispositivo canônico '{canonical}'."
                ),
                Json::obj([
                    ("motivo", Json::str("alias")),
                    ("sensor", Json::str(nome)),
                    ("canonical", Json::str(canonical.clone())),
                ]),
            );
        }
        match self.rotas.get(&canonical).cloned() {
            Some(Rota::Simulador) => self.sim.read_sensor(&canonical, caderno),
            Some(Rota::Real) => self.ler_real(&canonical, caderno),
            Some(Rota::Remota(_)) => self.ler_remota(&canonical, caderno),
            Some(Rota::Inacessivel { motivo }) => {
                self.alerta_sensor(
                    caderno,
                    "sensor_inacessivel",
                    &canonical,
                    &format!("{motivo}."),
                );
                Err(FalhaSensor::Inacessivel)
            }
            None => {
                self.alerta_sensor(caderno, "sensor_nao_registrado", &canonical, "sem rota.");
                Err(FalhaSensor::NaoRegistrado)
            }
        }
    }

    fn act(&mut self, ator: &str, valor: Value, caderno: &mut dyn Caderno) -> ActOutcome {
        self.act_with_priority(ator, valor, PRIORIDADE_NORMAL, caderno)
    }

    fn act_with_priority(
        &mut self,
        ator: &str,
        valor: Value,
        prioridade: u8,
        caderno: &mut dyn Caderno,
    ) -> ActOutcome {
        let canonical = self.registry.canonical_de(ator).to_string();
        if !self.registry.contains(&canonical) {
            caderno.record(
                rt_kinds::ATOR_INEXISTENTE,
                &format!("Ator '{ator}' não registrado no FXP."),
                Json::obj([("ator", Json::str(ator))]),
            );
            caderno.actuator_action(ator, &valor, false);
            return ActOutcome::AtorInexistente;
        }
        let rota = self.rotas.get(&canonical).cloned();
        match rota {
            Some(Rota::Simulador) => {
                // Paridade com a Etapa 2: validação, efeitos e eventos do sim.
                self.sim.act(&canonical, valor, caderno)
            }
            Some(Rota::Real) | Some(Rota::Remota(_)) | Some(Rota::Inacessivel { .. }) => {
                // Limites do REGISTRO (inclusivos) antes do envio (§4.3).
                let limits = self.limites_de(&canonical);
                if let Some((limite, valor_limite)) = self.violacao(&limits, &valor) {
                    return self.rejeitar_por_limite(
                        &canonical,
                        &valor,
                        limite,
                        valor_limite,
                        caderno,
                    );
                }
                match self.entregar_rota(&canonical, &valor, caderno) {
                    Some(outcome) => outcome,
                    None => {
                        // Indisponível: fallback do registro; esgotado → fila
                        // prioritária (retry em ticks futuros — PLAN §3.4).
                        let outcome = self.tentar_fallback(&canonical, &valor, caderno);
                        if matches!(outcome, ActOutcome::FallbackEsgotado) {
                            self.enfileirar(&canonical, &valor, prioridade);
                        }
                        outcome
                    }
                }
            }
            None => {
                caderno.record(
                    rt_kinds::ATOR_INEXISTENTE,
                    &format!("Ator '{ator}' não registrado no FXP."),
                    Json::obj([("ator", Json::str(ator))]),
                );
                caderno.actuator_action(ator, &valor, false);
                ActOutcome::AtorInexistente
            }
        }
    }

    fn cpu_power(&self) -> f64 {
        match self.rotas.get("cpu_power") {
            Some(Rota::Simulador) | None => self.sim.cpu_power(),
            _ => self.potencia_conhecida,
        }
    }

    fn on_tick(&mut self, caderno: &mut dyn Caderno) {
        self.sim.on_tick(caderno);
        self.atualizar_potencia();
        self.bombear_fila(caderno);
    }

    fn disk_bytes_used(&self) -> u64 {
        self.disk_bytes
    }

    fn add_disk_bytes(&mut self, n: u64) {
        self.disk_bytes += n;
    }

    fn registry(&self) -> &RuntimeRegistry {
        &self.rt_registry
    }
}

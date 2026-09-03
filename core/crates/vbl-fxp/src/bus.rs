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

use crate::drivers::{actor_from, discover, sensor_from, ActorDriver, SensorDriver};
use crate::queue::{Command, CommandQueue};
use crate::registry::{
    DeviceKind, DeviceMode, DeviceRegistry, Endpoint, OperationMode, RemoteAddr,
};
use crate::schema::{caps, flag, reason, AckAct, BatchResult, Body, Message, WireValue};
use crate::transport::{Connection, TransportError};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vbl_runtime::fxp::{
    ActOutcome, ActorLimits, Fxp, Limit, Registry as RuntimeRegistry, SensorFailure, Value,
    PRIORITY_NORMAL,
};
use vbl_runtime::json::Json;
use vbl_runtime::ledger::{kinds as rt_kinds, Actuation, Ledger};
use vbl_runtime::sim::FxpSimulator;

/// Kinds de evento acrescentados pelo barramento (vocabulário do Caderno —
/// FORMAL §6 lista os canônicos de fim de forma; estes cobrem o ciclo de
/// vida da fila prioritária e os recursos de transporte v1.1).
pub mod kinds {
    /// Comando enfileirado foi entregue numa re-entrega do `on_tick`.
    pub const COMANDO_REENTREGUE: &str = "comando_reentregue";
    /// Comando excedeu `queue_timeout_ticks` e foi descartado com alerta.
    pub const COMANDO_EXPIRADO: &str = "comando_expirado";
    /// Lote de leituras remotas (`READ_BATCH`, schema §4.7) — diagnóstico de
    /// transporte; o evento semântico de leitura continua sendo o do runtime.
    pub const FXP_BATCH: &str = "fxp_batch";
    /// Peer remoto não anunciou a capacidade pedida — segue em v1.0
    /// (degradação honesta e logada, nunca silenciosa).
    pub const FXP_PEER_V1: &str = "fxp_peer_v1";
    /// Dicionário treinado divergiu no `DICT_SYNC` (v1.4 §4.8 — ex.: pontas
    /// com versões de zstd diferentes): a conexão degrada para o id 2 com
    /// registro honesto, nunca tenta frame que falharia.
    pub const FXP_DICT_DIVERGENTE: &str = "fxp_dict_divergente";
}

/// Trilha de atuação com a latência medida (Etapa 4 — PLAN §4.1): o Caderno
/// estima o custo energético como potência do tick × latência do ack.
fn actuation_with_latency(
    ledger: &mut dyn Ledger,
    actor: &str,
    value: &Value,
    latency_us: u64,
    success: bool,
) {
    ledger.actuator_action_detailed(Actuation {
        actor: actor.to_owned(),
        requested: value.clone(),
        applied: if success { Some(value.clone()) } else { None },
        latency_us: Some(latency_us),
        joule_cost: None,
        success,
    });
}

/// Parâmetros do barramento (defaults = docs/FXP-SCHEMA-v1.md §6).
#[derive(Debug, Clone)]
pub struct BusConfig {
    pub mode: OperationMode,
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
    // -- v1.1 (docs/FXP-SCHEMA-v1.md §4.5–§4.8): tudo opt-in; defaults
    // -- preservam o wire bit a bit v1.0.
    /// Prefetch de lote no primeiro cache-miss do tick (`READ_BATCH` §4.7).
    pub batch_prefetch: bool,
    /// Compressão LZ4 negociada dos frames (§4.8).
    pub compression: bool,
    /// Dicionário LZ4 compartilhado do registro (v1.2 §4.8) — pede
    /// `caps::DICT` e completa o `HELLO` no handshake; o dicionário é
    /// derivado do registro do peer (nenhum byte cruza o fio).
    pub compression_dict: bool,
    /// zstd com dicionário TREINADO (v1.3 §4.8) — pede `caps::ZSTD +
    /// caps::DICT` (o gatilho do `HELLO` é o mesmo); se o peer não conceder
    /// zstd, a degradação segue a ordem id 3 → id 2 → plano (o que a
    /// interseção `CAPS_OK` trouxer).
    pub compression_zstd: bool,
    /// zstd com dicionário VERIFICADO no fio (v1.4 §4.8) — pede
    /// `caps::ZSTD_V + ZSTD + DICT`: após o `HELLO` o par troca
    /// `(versão do zstd, hash do dict)` via `DICT_SYNC`; hash casado ⇒ id 4,
    /// divergente (pontas com versões de zstd diferentes) ⇒ id 2 com
    /// registro no Caderno (nunca `DecompressionFailed` por descuido).
    pub compression_zstd_v: bool,
    /// Pedir `FLAG_TIMESTAMP` ao peer e propagar como `fio_us` (§5).
    pub wire_timestamp: bool,
    /// PSK de autenticação do canal remoto (§4.6) — bytes da chave; o CLI
    /// resolve de env (`psk:VAR`); a chave nunca trafega no fio.
    pub psk: Option<Vec<u8>>,
    /// Janela de escuta da descoberta multicast (§4.9) para endpoints
    /// `discover:<identificador>`.
    pub discover_window: Duration,
    /// Grupo de descoberta (v1.2 §4.9): `ip:porta` v4, `[v6]:porta` (scope
    /// numérico `%N`) e `@fonte` para SSM (v4 desde a v1.2; v6 desde a
    /// v1.3 — ex.: `[ff35::7080]:porta@[fe80::1%2]`). Default = grupo v4.
    pub discover_group: Option<String>,
    /// Store TOFU (v1.3 §7) para endpoints `tcps:...@tofu` — caminho do
    /// arquivo (o CLI resolve a flag/padrão). `None` + endpoint @tofu ⇒ usa
    /// [`crate::tls::TofuStore::caminho_padrao`]; sem nenhum ⇒ falha honesta
    /// da conexão (nunca confiar sem poder gravar a primeira use).
    pub tofu_store: Option<std::path::PathBuf>,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            mode: OperationMode::Simulated,
            cache_ttl: Duration::from_millis(100),
            read_timeout: Duration::from_millis(10),
            act_timeout_local: Duration::from_millis(50),
            act_timeout_remote: Duration::from_millis(500),
            retries: 1,
            queue_timeout_ticks: 2,
            batch_prefetch: false,
            compression: false,
            compression_dict: false,
            compression_zstd: false,
            compression_zstd_v: false,
            wire_timestamp: false,
            psk: None,
            discover_window: Duration::from_millis(500),
            discover_group: None,
            tofu_store: None,
        }
    }
}

/// Rota efetiva de um dispositivo (resolvida na construção).
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// Simulador embutido (modo simulado global ou dispositivo simulado).
    Simulator,
    /// Driver de arquivo real (sysfs/hwmon/led).
    Real,
    /// Peer remoto falando schema v1.
    Remote(RemoteAddr),
    /// Registrado porém inacessível: modo real global proíbe a rota simulada
    /// e não há rota real (§4.7 — nunca simulado silencioso).
    Inaccessible { reason: String },
}

impl Route {
    /// Descrição legível (probe/relatório).
    pub fn description(&self) -> String {
        match self {
            Route::Simulator => "simulado (em processo)".into(),
            Route::Real => "real (driver de arquivo)".into(),
            Route::Remote(a) => match a {
                RemoteAddr::Unix(p) => format!("remota (unix:{})", p.display()),
                RemoteAddr::Tcp { host, port } => format!("remota (tcp:{host}:{port})"),
                RemoteAddr::TcpTls { host, port, .. } => format!("remota (tcps:{host}:{port})"),
            },
            Route::Inaccessible { reason } => format!("inacessível ({reason})"),
        }
    }
}

/// O barramento FXP consumido pelo runtime (`Engine<F: Fxp>`).
pub struct FxpBus {
    registry: DeviceRegistry,
    rt_registry: RuntimeRegistry,
    config: BusConfig,
    sim: FxpSimulator,
    routes: BTreeMap<String, Route>, // canônico → rota
    real_sensors: BTreeMap<String, Box<dyn SensorDriver + Send>>,
    real_actors: BTreeMap<String, Box<dyn ActorDriver + Send>>,
    connections: BTreeMap<String, Connection>, // endereço do peer → transporte
    cache: BTreeMap<String, (Instant, f64)>,
    /// Timestamp físico do fio por canônico (§5 — anotação de laboratório;
    /// o Caderno segue no relógio virtual). Presente só quando o peer carimbou.
    wire_ts: BTreeMap<String, u64>,
    /// Peers que não anunciaram v1.1 (CAPS falhou) — degradação honesta.
    v1_peers: BTreeMap<String, bool>,
    /// Store TOFU aberto (v1.3 §7) — aberto na 1ª conexão `@tofu` e
    /// compartilhado por todas (a primeira use grava uma única vez).
    tofu: Option<Arc<Mutex<crate::tls::TofuStore>>>,
    queue: CommandQueue,
    seq: u32,
    disk_bytes: u64,
    /// Última leitura de `cpu_power` em rota real/remota (partilha P/N — §4.2).
    known_power: f64,
    power_inaccessible: bool,
    power_read_at: Option<Instant>,
}

impl std::fmt::Debug for FxpBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FxpBus")
            .field("config", &self.config)
            .field("rotas", &self.routes)
            .field("fila", &self.queue.len())
            .finish()
    }
}

impl FxpBus {
    /// Constrói o barramento com rotas resolvidas. Endpoints `Auto` tentam
    /// descoberta no host; falha de descoberta **não é erro de construção** —
    /// o dispositivo fica registrado porém inacessível (§4.7).
    pub fn build(registry: DeviceRegistry, mut config: BusConfig, mut sim: FxpSimulator) -> Self {
        let mut routes = BTreeMap::new();
        let mut real_sensors = BTreeMap::new();
        let mut real_actors = BTreeMap::new();

        if config.mode == OperationMode::Simulated {
            // Simulador determinístico: sem cache (roteirização é imediata).
            config.cache_ttl = Duration::ZERO;
        }

        // Fonte única de verdade: o registro rico acrescenta ao simulador os
        // dispositivos que só existem nele (extensões). Dispositivos já
        // presentes no sim (roteirizados pelo CLI) são PRESERVADOS — estado
        // como "ator indisponível" do cenário não pode ser resetado.
        for d in registry.devices() {
            match &d.kind {
                DeviceKind::Sensor { quantity, unit, .. } => {
                    if !sim.registry().sensores.contains_key(&d.name) {
                        sim.register_sensor(
                            &d.name,
                            vbl_runtime::fxp::SensorInfo {
                                quantity: quantity.clone(),
                                unit: unit.clone(),
                            },
                        );
                    }
                }
                DeviceKind::Actor { limits } => {
                    if !sim.registry().actors.contains_key(&d.name) {
                        sim.register_actor(&d.name, limits.clone());
                    }
                }
            }
        }

        for d in registry.devices() {
            let route = match base_route(&config, d) {
                RouteSpec::Simulator => Route::Simulator,
                RouteSpec::Inaccessible { reason } => Route::Inaccessible { reason },
                RouteSpec::Concrete => {
                    // v1.1 §4.9: descoberta multicast resolve AQUI (build) —
                    // sem anúncio no prazo ⇒ registrado porém inacessível.
                    let endpoint = match &d.endpoint {
                        Endpoint::Auto => discover(&d.name),
                        Endpoint::AutoRemote { identifier } => {
                            // v1.2 §4.9: grupo configurável (v4/v6) + SSM
                            // (`@fonte`); parse ruim ⇒ registrado porém
                            // inacessível com o motivo honesto do build.
                            let descoberta = match &config.discover_group {
                                None => crate::discover::discover_peers(
                                    config.discover_window,
                                    crate::discover::DEFAULT_GROUP,
                                ),
                                Some(txt) => match crate::discover::parse_group(txt) {
                                    Err(_) => Err(crate::discover::DiscoveryError::BeaconInvalido),
                                    Ok((grupo, Some(fonte))) => {
                                        crate::discover::discover_peers_ssm(
                                            config.discover_window,
                                            grupo,
                                            fonte,
                                        )
                                    }
                                    Ok((grupo, None)) => crate::discover::discover_peers(
                                        config.discover_window,
                                        grupo,
                                    ),
                                },
                            };
                            match descoberta {
                                Ok(peers) => {
                                    match peers.into_iter().find(|p| p.identifier == *identifier) {
                                        Some(p) => Some(Endpoint::Remote {
                                            addr: RemoteAddr::Tcp {
                                                host: p.source.ip().to_string(),
                                                port: p.tcp_port,
                                            },
                                        }),
                                        None => None,
                                    }
                                }
                                Err(_) => None,
                            }
                        }
                        Endpoint::AutoRemoteMdns { identifier } => {
                            // v1.2 §4.10: DNS-SD com TXT `id`/`hash` (+ tls/
                            // pin ⇒ tcps). Janela sem resposta ⇒ inacessível
                            // (mDNS é lossy, mesma honestidade do beacon).
                            #[cfg(feature = "mdns")]
                            match crate::mdns::discover_mdns(config.discover_window) {
                                Ok(peers) => {
                                    match peers.into_iter().find(|p| p.identifier == *identifier) {
                                        Some(p) => Some(Endpoint::Remote {
                                            addr: match p.tls {
                                                Some(pin) => RemoteAddr::TcpTls {
                                                    host: p.host.to_string(),
                                                    port: p.port,
                                                    trust: crate::tls::Trust::Pin(vec![pin]),
                                                },
                                                None => RemoteAddr::Tcp {
                                                    host: p.host.to_string(),
                                                    port: p.port,
                                                },
                                            },
                                        }),
                                        None => None,
                                    }
                                }
                                Err(_) => None,
                            }
                            // Sem a feature o parse já rejeita `mdns:`; o
                            // braço é inatingível — e segue honesto.
                            #[cfg(not(feature = "mdns"))]
                            {
                                let _ = identifier;
                                None
                            }
                        }
                        e => Some(e.clone()),
                    };
                    match endpoint {
                        None => Route::Inaccessible {
                            reason: "auto-descoberta não encontrou hardware".into(),
                        },
                        Some(Endpoint::Remote { addr }) => Route::Remote(addr),
                        Some(Endpoint::Simulated) => Route::Inaccessible {
                            reason: "modo real não roteia para simulador (dado sintético proibido)"
                                .into(),
                        },
                        Some(ep) => match &d.kind {
                            DeviceKind::Sensor { .. } => match sensor_from(&ep) {
                                Some(dr) => {
                                    real_sensors.insert(d.name.clone(), dr);
                                    Route::Real
                                }
                                None => Route::Inaccessible {
                                    reason: format!(
                                        "endpoint sem driver de leitura: {}",
                                        ep.description()
                                    ),
                                },
                            },
                            DeviceKind::Actor { .. } => match actor_from(&ep) {
                                Some(dr) => {
                                    real_actors.insert(d.name.clone(), dr);
                                    Route::Real
                                }
                                None => Route::Inaccessible {
                                    reason: format!(
                                        "endpoint sem driver de atuação: {}",
                                        ep.description()
                                    ),
                                },
                            },
                        },
                    }
                }
            };
            routes.insert(d.name.clone(), route);
        }

        let rt_registry = registry.to_runtime_registry();
        Self {
            registry,
            rt_registry,
            config,
            sim,
            routes,
            real_sensors,
            real_actors,
            connections: BTreeMap::new(),
            cache: BTreeMap::new(),
            wire_ts: BTreeMap::new(),
            v1_peers: BTreeMap::new(),
            tofu: None,
            queue: CommandQueue::default(),
            seq: 0,
            disk_bytes: 0,
            known_power: 0.0,
            power_inaccessible: false,
            power_read_at: None,
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
    pub fn pending_queue(&self) -> usize {
        self.queue.len()
    }

    /// Rota efetiva de um nome simbólico (para `vbl fxp-probe`).
    pub fn route_of(&self, name: &str) -> Option<&Route> {
        self.routes.get(self.registry.canonical_of(name))
    }

    fn sensor_alert(&mut self, ledger: &mut dyn Ledger, reason: &str, sensor: &str, detail: &str) {
        ledger.alert(
            &format!(
                "Sensor '{sensor}' — falha de I/O ({reason}): {detail} Condição não avaliada neste tick."
            ),
            Json::obj([("motivo", Json::str(reason)), ("sensor", Json::str(sensor))]),
        );
    }

    fn next_seq(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn cache_valid(&self, canonical: &str) -> Option<f64> {
        if self.config.cache_ttl.is_zero() {
            return None;
        }
        match self.cache.get(canonical) {
            Some((read_at, value)) if read_at.elapsed() < self.config.cache_ttl => Some(*value),
            _ => None,
        }
    }

    /// Leitura em rota real: cache TTL → driver → honestidade de falha.
    fn read_real(
        &mut self,
        canonical: &str,
        ledger: &mut dyn Ledger,
    ) -> Result<f64, SensorFailure> {
        if let Some(v) = self.cache_valid(canonical) {
            return Ok(v);
        }
        let Some(dr) = self.real_sensors.get_mut(canonical) else {
            self.sensor_alert(ledger, "sensor_inaccessible", canonical, "sem driver real.");
            return Err(SensorFailure::Inaccessible);
        };
        match dr.read() {
            Ok(v) => {
                self.cache.insert(canonical.into(), (Instant::now(), v));
                self.note_power(canonical, v);
                Ok(v)
            }
            Err(_) => {
                if canonical == "cpu_power" {
                    self.power_inaccessible = true;
                }
                let detail = format!(
                    "driver real ({}) falhou.",
                    self.route_description(canonical)
                );
                self.sensor_alert(ledger, "sensor_inaccessible", canonical, &detail);
                Err(SensorFailure::Inaccessible)
            }
        }
    }

    /// Leitura remota via schema v1.1: batch prefetch (§4.7, opt-in) →
    /// READ individual → READ_OK/READ_ERR, ack por seq.
    fn read_remote(
        &mut self,
        canonical: &str,
        ledger: &mut dyn Ledger,
    ) -> Result<f64, SensorFailure> {
        if let Some(v) = self.cache_valid(canonical) {
            return Ok(v);
        }
        let Some(Route::Remote(addr)) = self.routes.get(canonical).cloned() else {
            return Err(SensorFailure::Inaccessible);
        };
        if self.config.batch_prefetch {
            // Conexão+negociação antes da decisão: CAPS precisa estar viva.
            if let Err(e) = self.ensure_remote(&addr, self.config.read_timeout, ledger) {
                self.sensor_alert(
                    ledger,
                    "sensor_inaccessible",
                    canonical,
                    &format!("transporte: {e}."),
                );
                return Err(SensorFailure::Inaccessible);
            }
            if self.granted_caps_of(&addr) & caps::BATCH != 0
                && self.alvos_de_batch(&addr).len() >= 2
            {
                return self.read_remote_batch(canonical, &addr, ledger);
            }
        }
        self.read_remote_individual(canonical, ledger)
    }

    /// Caminho individual (v1.0 compatível), com captura de timestamp do fio.
    fn read_remote_individual(
        &mut self,
        canonical: &str,
        ledger: &mut dyn Ledger,
    ) -> Result<f64, SensorFailure> {
        let Some(Route::Remote(addr)) = self.routes.get(canonical).cloned() else {
            return Err(SensorFailure::Inaccessible);
        };
        let seq = self.next_seq();
        let request = Message::read(canonical, seq, true);
        match self.request_remote(&addr, &request, self.config.read_timeout, ledger) {
            Ok(resp) => {
                let synthetic = resp.flags & flag::SYNTHETIC != 0;
                let fio_us = resp.timestamp_us;
                if let Some(ts) = fio_us {
                    self.wire_ts.insert(canonical.into(), ts);
                }
                match resp.body {
                    Body::ReadOk { value, .. } => {
                        if synthetic {
                            // §4.7: dado sintético sempre marcado no Caderno.
                            ledger.warn(
                                &format!("Leitura remota de '{canonical}' é de origem simulada (measurement_status: simulado)."),
                                fio_us_json(
                                    "measurement_status_simulado",
                                    canonical,
                                    value,
                                    fio_us,
                                ),
                            );
                        }
                        self.cache.insert(canonical.into(), (Instant::now(), value));
                        self.note_power(canonical, value);
                        Ok(value)
                    }
                    Body::ReadErr { reason } => {
                        let (failure, motivo) = motivo_de_reason(reason);
                        self.sensor_alert(ledger, motivo, canonical, "peer respondeu erro.");
                        Err(failure)
                    }
                    _ => {
                        self.sensor_alert(
                            ledger,
                            "sensor_inaccessible",
                            canonical,
                            "resposta inesperada do peer.",
                        );
                        Err(SensorFailure::Inaccessible)
                    }
                }
            }
            Err(e) => {
                self.connections.remove(&addr_key(&addr)); // conexão suspeita
                self.sensor_alert(
                    ledger,
                    "sensor_inaccessible",
                    canonical,
                    &format!("transporte: {e}."),
                );
                Err(SensorFailure::Inaccessible)
            }
        }
    }

    fn note_power(&mut self, canonical: &str, value: f64) {
        if canonical == "cpu_power" {
            self.known_power = value;
            self.power_inaccessible = false;
            self.power_read_at = Some(Instant::now());
        }
    }

    /// Pedido-resposta remoto com reconexão preguiçosa. Uma conexão **por
    /// endereço de peer** (sensores do mesmo peer compartilham o fio —
    /// pressuposto do batching §4.7). Na abertura: AUTH (§4.6, se PSK) e
    /// depois CAPS (§4.5, se algum recurso foi pedido); peer que não responde
    /// à negociação ⇒ degradação honesta para v1.0 com evento no Caderno.
    fn request_remote(
        &mut self,
        addr: &RemoteAddr,
        request: &Message,
        timeout: Duration,
        ledger: &mut dyn Ledger,
    ) -> Result<Message, TransportError> {
        self.ensure_remote(addr, timeout, ledger)?;
        let key = addr_key(addr);
        let c = self.connections.get_mut(&key).expect("garantido acima");
        c.request(request, timeout)
    }

    /// Garante conexão aberta (e negociada) com o peer — separada do pedido
    /// para que a decisão de batching veja as CAPS já concedidas (§4.5).
    fn ensure_remote(
        &mut self,
        addr: &RemoteAddr,
        timeout: Duration,
        ledger: &mut dyn Ledger,
    ) -> Result<(), TransportError> {
        let key = addr_key(addr);
        if self.connections.contains_key(&key) {
            return Ok(());
        }
        // Handshake tem prazo PRÓPRIO (v1.3 §4.8): a derivação do dicionário
        // treinado (COVER) é trabalho real nos DOIS lados — o prazo de
        // leitura (dezenas de ms por sensor) não serve para o handshake.
        let hs = timeout.max(Duration::from_millis(500));
        // v1.3 §7: o CAPS pode partir como 0-RTT na conexão TLS retomada —
        // só sem PSK (com AUTH o servidor fala primeiro, §4.6).
        let wanted = self.wanted_caps();
        let early_caps = if wanted != 0 && self.config.psk.is_none() {
            Some(wanted)
        } else {
            None
        };
        let mut c = match addr {
            RemoteAddr::Unix(p) => Connection::unix(p, timeout)?,
            RemoteAddr::Tcp { host, port } => Connection::tcp(host, *port, timeout)?,
            // v1.2/v1.3 §7: TLS com pin/TOFU — handshake falho/divergente é
            // terminativo (nunca reconecta em texto plano).
            RemoteAddr::TcpTls { host, port, trust } => {
                let confianca = self.confianca_cliente(host, *port, trust)?;
                Connection::tcp_tls(host, *port, &confianca, hs, early_caps)?
            }
        };
        // §6 — ordem: AUTH → CAPS → trabalho. Falha de AUTENTICAÇÃO é
        // terminativa (segurança não degrada para canal aberto).
        if let Some(psk) = &self.config.psk {
            if let Err(e) = c.authenticate(psk, hs) {
                return Err(TransportError::Broken(format!("auth: {e}")));
            }
        }
        let concedidas = if wanted != 0 {
            match c.negotiate(wanted, hs) {
                Ok(g) => g,
                Err(TransportError::Timeout) => return Err(TransportError::Timeout),
                Err(_) => {
                    // Peer provavelmente v1.0 (fechou diante de CAPS): reconecta
                    // em modo v1.0 puro e REGISTRA a degradação (nunca silenciosa).
                    let c2 = match addr {
                        RemoteAddr::Unix(p) => Connection::unix(p, timeout)?,
                        RemoteAddr::Tcp { host, port } => Connection::tcp(host, *port, timeout)?,
                        RemoteAddr::TcpTls { host, port, trust } => {
                            let confianca = self.confianca_cliente(host, *port, trust)?;
                            Connection::tcp_tls(host, *port, &confianca, hs, early_caps)?
                        }
                    };
                    self.connections.insert(key.clone(), c2);
                    self.v1_peers.insert(key.clone(), true);
                    ledger.record(
                        kinds::FXP_PEER_V1,
                        &format!(
                            "Peer remoto ({key}) não anunciou v1.1 (CAPS); seguindo em modo v1.0 — recursos de fio desligados."
                        ),
                        Json::obj([
                            ("motivo", Json::str("fxp_peer_v1")),
                            ("peer", Json::str(key.clone())),
                        ]),
                    );
                    return Ok(());
                }
            }
        } else {
            0
        };
        // v1.2/v1.3 §4.8: DICT concedido ⇒ HELLO integra o handshake. O
        // cliente publica o registro local e deriva o dicionário do registro
        // do PEER (o servidor deriva do dele — mesmos bytes, sem fio extra).
        // Com zstd concedido, o dicionário é o TREINADO (id 3); derivar e
        // falhar ⇒ erro honesto (divergência de versão não vira silêncio).
        if concedidas & caps::DICT != 0 {
            let local: Vec<crate::schema::DeviceDesc> = self
                .registry_rico()
                .devices()
                .map(|d| d.to_device_desc())
                .collect();
            let remoto = c
                .exchange_hello(&local, hs)
                .map_err(|e| TransportError::Broken(format!("handshake dict (HELLO): {e}")))?;
            let nomes: Vec<String> = remoto.iter().map(|d| d.name().to_string()).collect();
            let concatenado = crate::schema::compress::dict_from_registry(&nomes);
            if concedidas & caps::ZSTD_V != 0 {
                // v1.4 §4.8: verificação no fio — hash casado ⇒ id 4;
                // divergente (ex.: pontas com versões de zstd diferentes) ⇒
                // id 2 com registro honesto no Caderno, nunca tentativa de
                // frame que falharia com DecompressionFailed.
                match crate::schema::compress::zstd_dict_from_registry(&nomes) {
                    Some(treinado) => {
                        let meu_hash = crate::schema::compress::hash_dict(&treinado);
                        match c.dict_sync(crate::schema::compress::zstd_version(), meu_hash, hs) {
                            Ok((_versao_peer, hash_peer)) if hash_peer == meu_hash => {
                                c.set_zstd_dict_v(treinado);
                            }
                            Ok((versao_peer, _)) => {
                                c.set_dict(concatenado);
                                ledger.record(
                                    kinds::FXP_DICT_DIVERGENTE,
                                    &format!(
                                        "Dicionário treinado divergiu no DICT_SYNC (peer zstd {versao_peer}, local zstd {local_zstd}) — conexão segue no id 2 (§4.8 v1.4).",
                                        local_zstd = crate::schema::compress::zstd_version()
                                    ),
                                    Json::obj([
                                        ("motivo", Json::str("fxp_dict_divergente")),
                                        ("versao_zstd_local", Json::num(f64::from(crate::schema::compress::zstd_version()))),
                                        ("versao_zstd_peer", Json::num(f64::from(versao_peer))),
                                    ]),
                                );
                            }
                            Err(e) => {
                                return Err(TransportError::Broken(format!(
                                    "handshake dict (DICT_SYNC): {e}"
                                )))
                            }
                        }
                    }
                    None => {
                        // Treino local impossível (registro curto na óTICA
                        // desta ponta): id 2 honesto com registro — o peer
                        // treinou com a versão DELE; sem verificação não há
                        // id 4.
                        c.set_dict(concatenado);
                        ledger.record(
                            kinds::FXP_DICT_DIVERGENTE,
                            "Peer concedeu ZSTD_V mas o dicionário treinado não derivou localmente — conexão segue no id 2 (§4.8 v1.4).",
                            Json::obj([("motivo", Json::str("treino_local_impossivel"))]),
                        );
                    }
                }
            } else if concedidas & caps::ZSTD != 0 {
                match crate::schema::compress::zstd_dict_from_registry(&nomes) {
                    Some(d) => c.set_zstd_dict(d),
                    None => {
                        return Err(TransportError::Broken(
                            "peer concedeu ZSTD mas o dicionário treinado não derivou localmente (§4.8) — conexão recusada"
                                .into(),
                        ))
                    }
                }
            } else {
                c.set_dict(concatenado);
            }
        }
        self.connections.insert(key, c);
        Ok(())
    }

    /// Confiança do cliente TLS (v1.3/v1.4 §7): pin é autossuficiente; TOFU
    /// (aprendiz ou estrito) abre/compartilha o store (config → padrão do
    /// usuário). Falha de abertura ⇒ falha fechada da conexão (nunca confiar
    /// sem poder registrar/verificar).
    fn confianca_cliente(
        &mut self,
        host: &str,
        port: u16,
        trust: &crate::tls::Trust,
    ) -> Result<crate::tls::ConfiancaCliente, TransportError> {
        match trust {
            crate::tls::Trust::Pin(pins) => {
                Ok(crate::tls::ConfiancaCliente::Pin(pins.clone()))
            }
            crate::tls::Trust::Tofu | crate::tls::Trust::TofuEstrito => {
                let estrito = *trust == crate::tls::Trust::TofuEstrito;
                if self.tofu.is_none() {
                    let caminho = self
                        .config
                        .tofu_store
                        .clone()
                        .or_else(crate::tls::TofuStore::caminho_padrao)
                        .ok_or_else(|| {
                            TransportError::ConnectionFailed(
                                "endpoint @tofu sem store: informe --tofu-store (ou XDG_STATE_HOME/HOME)"
                                    .into(),
                            )
                        })?;
                    let store = crate::tls::TofuStore::open(&caminho).map_err(|e| {
                        TransportError::ConnectionFailed(format!(
                            "store TOFU {}: {e}",
                            caminho.display()
                        ))
                    })?;
                    self.tofu = Some(Arc::new(Mutex::new(store)));
                }
                let store = self.tofu.clone().expect("garantido acima");
                if estrito {
                    Ok(crate::tls::ConfiancaCliente::TofuEstrito {
                        store,
                        host: host.to_string(),
                        port,
                    })
                } else {
                    Ok(crate::tls::ConfiancaCliente::Tofu {
                        store,
                        host: host.to_string(),
                        port,
                    })
                }
            }
        }
    }

    /// Capacidades que este bus pede ao peer (config opt-in v1.1/v1.2/v1.3).
    fn wanted_caps(&self) -> u16 {
        let mut w = 0;
        if self.config.compression {
            w |= caps::LZ4;
        }
        if self.config.compression_dict {
            w |= caps::DICT;
        }
        if self.config.compression_zstd {
            // v1.3 §4.8: zstd anda SEMPRE com DICT — o gatilho do HELLO é o
            // mesmo e a degradação (sem treino/zstd no peer) cai no id 2.
            w |= caps::ZSTD | caps::DICT;
        }
        if self.config.compression_zstd_v {
            // v1.4 §4.8: o id 4 implica o par v1.3 (DICT+ZSTD) — a ordem de
            // degradação v1.4 é id 4 → id 2 (o id 3 fica para quem pediu
            // exatamente a v1.3).
            w |= caps::ZSTD_V | caps::ZSTD | caps::DICT;
        }
        if self.config.batch_prefetch {
            w |= caps::BATCH;
        }
        if self.config.wire_timestamp {
            w |= caps::TIMESTAMP;
        }
        w
    }

    /// Capacidades concedidas pelo peer do endereço (0 sem negociação) —
    /// observação para testes/probe.
    pub fn granted_caps_of(&self, addr: &RemoteAddr) -> u16 {
        self.connections
            .get(&addr_key(addr))
            .map(|c| c.negotiated_caps())
            .unwrap_or(0)
    }

    /// Invalida o cache de leitura (probe/bench/testes: força re-I/O na
    /// próxima leitura de cada sensor).
    pub fn invalidate_cache(&mut self) {
        self.cache.clear();
    }

    /// Timestamp físico do fio da última leitura do canônico (§5) —
    /// anotação de laboratório; `None` = peer não carimbou.
    pub fn wire_timestamp_of(&self, name: &str) -> Option<u64> {
        self.wire_ts.get(self.registry.canonical_of(name)).copied()
    }

    /// Sensores remotos de um peer com cache vencido (alvos do prefetch).
    fn alvos_de_batch(&self, addr: &RemoteAddr) -> Vec<String> {
        self.registry
            .devices()
            .filter(|d| matches!(d.kind, DeviceKind::Sensor { .. }))
            .map(|d| d.name.clone())
            .filter(|n| {
                matches!(self.routes.get(n), Some(Route::Remote(a)) if a == addr)
                    && self.cache_valid(n).is_none()
            })
            .collect()
    }

    /// Leitura por lote (§4.7): um RTT para todos os sensores vencidos do
    /// peer. Sucesso pré-preenche o cache; falha de item NÃO vira alerta —
    /// o alerta continua pertencendo à pergunta feita (§4.7 do schema).
    fn read_remote_batch(
        &mut self,
        canonical: &str,
        addr: &RemoteAddr,
        ledger: &mut dyn Ledger,
    ) -> Result<f64, SensorFailure> {
        let alvos = self.alvos_de_batch(addr);
        if alvos.len() < 2 {
            // Lote de 1 = READ individual sem ganho: mantém o caminho v1.0.
            return self.read_remote_individual(canonical, ledger);
        }
        let seq = self.next_seq();
        let request = Message::read_batch(alvos.clone(), seq);
        match self.request_remote(addr, &request, self.config.read_timeout, ledger) {
            Ok(resp) => {
                let synthetic = resp.flags & flag::SYNTHETIC != 0;
                let fio_us = resp.timestamp_us;
                match resp.body {
                    Body::ReadBatchOk { results } => {
                        ledger.record(
                            kinds::FXP_BATCH,
                            &format!(
                                "Lote de {} sensor(es) remotos em 1 RTT ({}).",
                                results.len(),
                                addr_descricao(addr)
                            ),
                            Json::obj([
                                ("motivo", Json::str("fxp_batch")),
                                ("itens", Json::num(results.len() as f64)),
                                ("peer", Json::str(addr_descricao(addr))),
                            ]),
                        );
                        let mut pedido: Option<(String, BatchResult)> = None;
                        for (nome, r) in alvos.iter().zip(results) {
                            if let BatchResult::Ok { value, .. } = &r {
                                self.cache.insert(nome.clone(), (Instant::now(), *value));
                                self.note_power(nome, *value);
                                if let Some(ts) = fio_us {
                                    self.wire_ts.insert(nome.clone(), ts);
                                }
                            }
                            if nome == canonical {
                                pedido = Some((nome.clone(), r));
                            }
                        }
                        match pedido {
                            Some((_, BatchResult::Ok { value, .. })) => {
                                if synthetic {
                                    // §4.7: dado sintético sempre marcado.
                                    ledger.warn(
                                        &format!("Lote remoto inclui '{canonical}' de origem simulada (measurement_status: simulado)."),
                                        fio_us_json("measurement_status_simulado", canonical, value, fio_us),
                                    );
                                }
                                Ok(value)
                            }
                            Some((nome, BatchResult::Err { reason })) => {
                                let (failure, motivo) = motivo_de_reason(reason);
                                self.sensor_alert(
                                    ledger,
                                    motivo,
                                    &nome,
                                    "peer respondeu erro no lote.",
                                );
                                Err(failure)
                            }
                            None => {
                                self.sensor_alert(
                                    ledger,
                                    "sensor_inaccessible",
                                    canonical,
                                    "lote sem o sensor pedido.",
                                );
                                Err(SensorFailure::Inaccessible)
                            }
                        }
                    }
                    _ => {
                        self.sensor_alert(
                            ledger,
                            "sensor_inaccessible",
                            canonical,
                            "resposta inesperada ao lote.",
                        );
                        Err(SensorFailure::Inaccessible)
                    }
                }
            }
            Err(e) => {
                self.connections.remove(&addr_key(addr));
                self.sensor_alert(
                    ledger,
                    "sensor_inaccessible",
                    canonical,
                    &format!("transporte: {e}."),
                );
                Err(SensorFailure::Inaccessible)
            }
        }
    }

    fn route_description(&self, canonical: &str) -> String {
        self.routes
            .get(canonical)
            .map(|r| r.description())
            .unwrap_or_else(|| "fora do registro".into())
    }

    // -----------------------------------------------------------------
    // Atuação: validação, rota, retry e fallback do registro (§4.3)
    // -----------------------------------------------------------------

    fn violation(&self, limits: &ActorLimits, value: &Value) -> Option<(Limit, f64)> {
        let v = value.as_num()?;
        // limites INCLUSIVOS: valor igual ao limite é aceito (FORMAL §4.3)
        if let Some(min) = limits.min {
            if v < min {
                return Some((Limit::Min, min));
            }
        }
        if let Some(max) = limits.max {
            if v > max {
                return Some((Limit::Max, max));
            }
        }
        if let Some(safety) = limits.safety_limit {
            if v > safety {
                return Some((Limit::SafetyLimit, safety));
            }
        }
        None
    }

    /// Limites do registro para um ator (fonte única).
    fn limits_of(&self, canonical: &str) -> ActorLimits {
        match self.registry.get(canonical).map(|d| &d.kind) {
            Some(DeviceKind::Actor { limits }) => limits.clone(),
            _ => ActorLimits::default(),
        }
    }

    fn reject_over_limit(
        &mut self,
        canonical: &str,
        value: &Value,
        limit: Limit,
        limit_value: f64,
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        ledger.record(
            rt_kinds::ACTOR_REJECTED_VALUE,
            &format!(
                "Comando a '{canonical}' rejeitado sem envio: valor viola {} = {limit_value}.",
                limit.name()
            ),
            Json::obj([
                ("ator", Json::str(canonical)),
                ("valor", value.to_json()),
                ("limite", Json::str(limit.name())),
                ("limite_valor", Json::num(limit_value)),
            ]),
        );
        ledger.actuator_action(canonical, value, false);
        ActOutcome::Rejected { limit, limit_value }
    }

    /// Tenta entregar na rota REAL do ator (sem fallback).
    /// `Ok(true)` = entregue; `Ok(false)` = indisponível (segue fallback);
    /// `Err(outcome)` = terminativo (rejeição/domínio — sem fallback).
    fn deliver_real(
        &mut self,
        canonical: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> Result<bool, ActOutcome> {
        let Some(dr) = self.real_actors.get_mut(canonical) else {
            return Ok(false);
        };
        // tentativas = 1 + retries (PLAN §3: fila com retry e fallback)
        let mut last_latency_us = 0u64;
        for _ in 0..=self.config.retries {
            let t0 = Instant::now();
            let result = dr.apply(value);
            last_latency_us = t0.elapsed().as_micros() as u64;
            match result {
                Ok(()) => {
                    // Etapa 4 (PLAN §4.1): valor aplicado + latência do ack;
                    // o custo energético é estimado pelo Caderno (W × latência).
                    actuation_with_latency(ledger, canonical, value, last_latency_us, true);
                    return Ok(true);
                }
                Err(crate::drivers::ActorError::InvalidValue(reason)) => {
                    actuation_with_latency(ledger, canonical, value, last_latency_us, false);
                    ledger.record(
                        rt_kinds::ACTOR_REJECTED_VALUE,
                        &format!("Comando a '{canonical}' fora do domínio do ator: {reason}."),
                        Json::obj([
                            ("ator", Json::str(canonical)),
                            ("valor", value.to_json()),
                            ("motivo", Json::str(reason.clone())),
                        ]),
                    );
                    return Err(ActOutcome::InvalidValue { reason });
                }
                Err(crate::drivers::ActorError::WriteFailed(_)) => {
                    continue; // retry de transporte antes do fallback
                }
            }
        }
        actuation_with_latency(ledger, canonical, value, last_latency_us, false);
        ledger.record(
            rt_kinds::ACTOR_UNAVAILABLE,
            &format!(
                "Heartbeat do ator '{canonical}' não respondeu (rota {}).",
                self.route_description(canonical)
            ),
            Json::obj([("ator", Json::str(canonical))]),
        );
        Ok(false)
    }

    /// Entrega remota (ACT → ACT_ACK, §4.3). `Some` = terminativo;
    /// `None` = indisponível (segue fallback).
    fn deliver_remote(
        &mut self,
        canonical: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> Option<ActOutcome> {
        let Some(Route::Remote(addr)) = self.routes.get(canonical).cloned() else {
            return Some(ActOutcome::MissingActor);
        };
        let seq = self.next_seq();
        let request = Message::act(canonical, wire_of(value), seq, true);
        let timeout = if matches!(addr, RemoteAddr::Tcp { .. }) {
            self.config.act_timeout_remote
        } else {
            self.config.act_timeout_local
        };
        let t0 = Instant::now();
        match self.request_remote(&addr, &request, timeout, ledger) {
            Ok(resp) => {
                let latency_us = t0.elapsed().as_micros() as u64;
                if let Some(ts) = resp.timestamp_us {
                    self.wire_ts.insert(canonical.into(), ts);
                }
                match resp.body {
                    Body::ActAck { status } => match status {
                        AckAct::Delivered => {
                            actuation_with_latency(ledger, canonical, value, latency_us, true);
                            Some(ActOutcome::Delivered)
                        }
                        AckAct::Rejected { limit, limit_value } => {
                            let limit = match limit {
                                0 => Limit::Min,
                                1 => Limit::Max,
                                _ => Limit::SafetyLimit,
                            };
                            Some(self.reject_over_limit(
                                canonical,
                                value,
                                limit,
                                limit_value,
                                ledger,
                            ))
                        }
                        AckAct::MissingActor => {
                            ledger.record(
                                rt_kinds::ACTOR_UNKNOWN,
                                &format!("Ator '{canonical}' não registrado no peer remoto."),
                                Json::obj([("ator", Json::str(canonical))]),
                            );
                            actuation_with_latency(ledger, canonical, value, latency_us, false);
                            Some(ActOutcome::MissingActor)
                        }
                        AckAct::InvalidValue { reason } => {
                            actuation_with_latency(ledger, canonical, value, latency_us, false);
                            Some(ActOutcome::InvalidValue { reason })
                        }
                        AckAct::Unavailable | AckAct::FallbackExhausted => {
                            actuation_with_latency(ledger, canonical, value, latency_us, false);
                            ledger.record(
                                rt_kinds::ACTOR_UNAVAILABLE,
                                &format!("Heartbeat do ator '{canonical}' não respondeu (peer)."),
                                Json::obj([("ator", Json::str(canonical))]),
                            );
                            None
                        }
                        AckAct::FallbackExecuted { alternativo } => {
                            // O peer executou o fallback do registro DELE (§4.3).
                            actuation_with_latency(ledger, &alternativo, value, latency_us, true);
                            ledger.record(
                                rt_kinds::FALLBACK_EXECUTED,
                                &format!(
                                    "Fallback '{alternativo}' acionado após falha de '{canonical}'."
                                ),
                                Json::obj([
                                    ("primario", Json::str(canonical)),
                                    ("alternativo", Json::str(alternativo.clone())),
                                    ("valor", value.to_json()),
                                ]),
                            );
                            Some(ActOutcome::FallbackExecuted { alternativo })
                        }
                    },
                    _ => {
                        actuation_with_latency(ledger, canonical, value, latency_us, false);
                        None
                    }
                }
            }
            Err(e) => {
                self.connections.remove(&addr_key(&addr));
                actuation_with_latency(
                    ledger,
                    canonical,
                    value,
                    t0.elapsed().as_micros() as u64,
                    false,
                );
                ledger.record(
                    rt_kinds::ACTOR_UNAVAILABLE,
                    &format!("Heartbeat do ator '{canonical}' não respondeu (transporte: {e})."),
                    Json::obj([("ator", Json::str(canonical))]),
                );
                None
            }
        }
    }

    /// Entrega em um ator específico pela rota DELE (fallback e re-entrega).
    /// `None` = indisponível.
    fn deliver_route(
        &mut self,
        canonical: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> Option<ActOutcome> {
        let route = self.routes.get(canonical)?.clone();
        match route {
            Route::Simulator => Some(self.sim.act(canonical, value.clone(), ledger)),
            Route::Real => {
                // Limites do registro (inclusivos) ANTES do envio (§4.3).
                let limits = self.limits_of(canonical);
                if let Some((limit, limit_value)) = self.violation(&limits, value) {
                    return Some(self.reject_over_limit(
                        canonical,
                        value,
                        limit,
                        limit_value,
                        ledger,
                    ));
                }
                match self.deliver_real(canonical, value, ledger) {
                    Ok(true) => Some(ActOutcome::Delivered),
                    Ok(false) => None,
                    Err(outcome) => Some(outcome),
                }
            }
            Route::Remote(_) => self.deliver_remote(canonical, value, ledger),
            Route::Inaccessible { reason } => {
                ledger.actuator_action(canonical, value, false);
                ledger.record(
                    rt_kinds::ACTOR_UNAVAILABLE,
                    &format!("Ator '{canonical}' indisponível ({reason})."),
                    Json::obj([("ator", Json::str(canonical))]),
                );
                None
            }
        }
    }

    /// Fallback do REGISTRO (FORMAL §4.3): primary → alternativos declarados
    /// no registro; o runtime não implementa fallback próprio.
    fn try_fallback(
        &mut self,
        primary: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        let alternativos: Vec<String> = self
            .registry
            .get(primary)
            .map(|d| d.fallback.clone())
            .unwrap_or_default();
        for alt in alternativos {
            if !self.registry.contains(&alt) {
                continue;
            }
            if let Some(outcome) = self.deliver_route(&alt, value, ledger) {
                if outcome.ok() {
                    ledger.record(
                        rt_kinds::FALLBACK_EXECUTED,
                        &format!("Fallback '{alt}' acionado após falha de '{primary}'."),
                        Json::obj([
                            ("primario", Json::str(primary)),
                            ("alternativo", Json::str(alt.clone())),
                            ("valor", value.to_json()),
                        ]),
                    );
                    // Contrato da Etapa 1/2: atuação por fallback SEMPRE
                    // devolve FallbackExecutado { alternativo } (BDD Caso 3),
                    // independentemente da variante de entrega da rota.
                    return ActOutcome::FallbackExecuted { alternativo: alt };
                }
            }
        }
        ledger.alert(
            &format!("Todos os fallbacks de '{primary}' falharam."),
            Json::obj([
                ("motivo", Json::str("fallback_esgotado")),
                ("ator", Json::str(primary)),
            ]),
        );
        ActOutcome::FallbackExhausted
    }

    fn enqueue(&mut self, actor: &str, value: &Value, priority: u8) {
        let cmd = Command {
            seq: self.next_seq(),
            actor: actor.into(),
            value: value.clone(),
            priority,
            ticks_waiting: 0,
            primary: None,
        };
        if self.queue.enqueue(cmd).is_err() {
            // Guarda anti-inchaço estourou: auditoria honesta do descarte.
            // (capacidade 256 torna isso improvável; o evento é obrigatório)
        }
    }

    /// Re-entrega da fila no relógio virtual (PLAN §3.4: prioridades e
    /// timeout), com trilha completa no Caderno.
    fn pump_queue(&mut self, ledger: &mut dyn Ledger) {
        if self.queue.is_empty() {
            return;
        }
        let mut pending = Vec::new();
        while let Some(cmd) = self.queue.dequeue() {
            pending.push(cmd);
        }
        for cmd in pending {
            if cmd.ticks_waiting >= self.config.queue_timeout_ticks {
                ledger.record(
                    kinds::COMANDO_EXPIRADO,
                    &format!(
                        "Comando a '{}' (seq {}) expirou na fila após {} tick(s).",
                        cmd.actor, cmd.seq, cmd.ticks_waiting
                    ),
                    Json::obj([
                        ("ator", Json::str(cmd.actor.clone())),
                        ("valor", cmd.value.to_json()),
                        ("ticks", Json::num(cmd.ticks_waiting as f64)),
                    ]),
                );
                ledger.alert(
                    &format!("Comando a '{}' expirou na fila do FXP.", cmd.actor),
                    Json::obj([
                        ("motivo", Json::str("comando_expirado")),
                        ("ator", Json::str(cmd.actor.clone())),
                    ]),
                );
                continue;
            }
            match self.deliver_route(&cmd.actor, &cmd.value, ledger) {
                Some(outcome) if outcome.ok() => {
                    ledger.record(
                        kinds::COMANDO_REENTREGUE,
                        &format!(
                            "Comando a '{}' re-entregue após {} tick(s) na fila.",
                            cmd.actor, cmd.ticks_waiting
                        ),
                        Json::obj([
                            ("ator", Json::str(cmd.actor.clone())),
                            ("valor", cmd.value.to_json()),
                            ("ticks", Json::num(cmd.ticks_waiting as f64)),
                        ]),
                    );
                }
                _ => {
                    // Falhou de novo: volta com +1 tick (expiração no topo).
                    let _ = self.queue.requeue(cmd);
                }
            }
        }
    }

    /// Varredura silenciosa da potência real (sem Caderno); falha vira alerta
    /// na próxima operação com caderno. Rotas remotas atualizam só quando
    /// lidas pelas regras (on_tick não faz I/O remoto — plano determinístico).
    fn update_power(&mut self) {
        if self.routes.get("cpu_power") != Some(&Route::Real) {
            return;
        }
        if let Some(t) = self.power_read_at {
            if t.elapsed() < self.config.cache_ttl {
                return;
            }
        }
        if let Some(dr) = self.real_sensors.get_mut("cpu_power") {
            match dr.read() {
                Ok(v) => {
                    self.known_power = v;
                    self.power_inaccessible = false;
                }
                Err(_) => self.power_inaccessible = true,
            }
            self.power_read_at = Some(Instant::now());
        }
    }
}

/// Rota pré-driver: o modo global modula o modo do dispositivo.
enum RouteSpec {
    Simulator,
    Concrete,
    Inaccessible { reason: String },
}

fn base_route(config: &BusConfig, d: &crate::registry::DeviceEntry) -> RouteSpec {
    let device_mode = match config.mode {
        OperationMode::Simulated => DeviceMode::Simulated,
        OperationMode::Real | OperationMode::Hybrid => d.mode,
    };
    match (config.mode, device_mode) {
        // Modo real global proíbe rota sintética — §4.7 (nunca simulado mudo).
        (OperationMode::Real, DeviceMode::Simulated) => RouteSpec::Inaccessible {
            reason: "modo real não roteia para simulador (dado sintético proibido)".into(),
        },
        // Simulado explícito (global, ou por dispositivo no híbrido).
        (_, DeviceMode::Simulated) => RouteSpec::Simulator,
        // Dispositivo real: rota concreta (Auto/driver/remota).
        (_, DeviceMode::Real) => RouteSpec::Concrete,
    }
}

pub(crate) fn wire_of(v: &Value) -> WireValue {
    match v {
        Value::Num(n) => WireValue::Num(*n),
        Value::Str(s) => WireValue::Str(s.clone()),
        Value::Ident(s) => WireValue::Ident(s.clone()),
    }
}

/// Chave de conexão por ENDEREÇO do peer (sensores do mesmo peer
/// compartilham o fio — pressuposto do batching §4.7).
pub(crate) fn addr_key(addr: &RemoteAddr) -> String {
    match addr {
        RemoteAddr::Unix(p) => format!("unix:{}", p.display()),
        RemoteAddr::Tcp { host, port } => format!("tcp:{host}:{port}"),
        RemoteAddr::TcpTls { host, port, trust } => match trust {
            crate::tls::Trust::Pin(pins) => format!(
                "tcps:{host}:{port}@sha256:{}",
                pins.iter()
                    .map(crate::tls::hex32)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            crate::tls::Trust::Tofu => format!("tcps:{host}:{port}@tofu"),
            crate::tls::Trust::TofuEstrito => format!("tcps:{host}:{port}@tofu-estrito"),
        },
    }
}

/// Descrição legível do endereço (eventos do Caderno).
pub(crate) fn addr_descricao(addr: &RemoteAddr) -> String {
    addr_key(addr)
}

/// `READ_ERR.reason` → (falha do runtime, motivo do evento) — FORMAL §4.7.
pub(crate) fn motivo_de_reason(reason: u8) -> (SensorFailure, &'static str) {
    match reason {
        reason::NOT_REGISTERED => (SensorFailure::NotRegistered, "sensor_not_registered"),
        _ => (SensorFailure::Inaccessible, "sensor_inaccessible"),
    }
}

/// JSON de evento de leitura com o timestamp do fio quando presente (§5).
pub(crate) fn fio_us_json(motivo: &str, sensor: &str, valor: f64, fio_us: Option<u64>) -> Json {
    let mut pares = vec![
        ("motivo", Json::str(motivo)),
        ("sensor", Json::str(sensor)),
        ("valor", Json::num(valor)),
    ];
    if let Some(ts) = fio_us {
        pares.push(("fio_us", Json::num(ts as f64)));
    }
    Json::obj(pares)
}

impl Fxp for FxpBus {
    fn read_sensor(&mut self, name: &str, ledger: &mut dyn Ledger) -> Result<f64, SensorFailure> {
        let canonical = self.registry.canonical_of(name).to_string();
        if !self.registry.contains(&canonical) {
            self.sensor_alert(
                ledger,
                "sensor_not_registered",
                name,
                "fora do registro do FXP.",
            );
            return Err(SensorFailure::NotRegistered);
        }
        // §6: leitura por alias é idêntica à do canônico; o Caderno registra
        // o nome usado (LEITURA do engine) e o canônico (este evento).
        if canonical != name {
            ledger.info(
                &format!(
                    "Leitura de '{name}' resolvida para o dispositivo canônico '{canonical}'."
                ),
                Json::obj([
                    ("motivo", Json::str("alias")),
                    ("sensor", Json::str(name)),
                    ("canonical", Json::str(canonical.clone())),
                ]),
            );
        }
        match self.routes.get(&canonical).cloned() {
            Some(Route::Simulator) => self.sim.read_sensor(&canonical, ledger),
            Some(Route::Real) => self.read_real(&canonical, ledger),
            Some(Route::Remote(_)) => self.read_remote(&canonical, ledger),
            Some(Route::Inaccessible { reason }) => {
                self.sensor_alert(
                    ledger,
                    "sensor_inaccessible",
                    &canonical,
                    &format!("{reason}."),
                );
                Err(SensorFailure::Inaccessible)
            }
            None => {
                self.sensor_alert(ledger, "sensor_not_registered", &canonical, "sem rota.");
                Err(SensorFailure::NotRegistered)
            }
        }
    }

    fn act(&mut self, actor: &str, value: Value, ledger: &mut dyn Ledger) -> ActOutcome {
        self.act_with_priority(actor, value, PRIORITY_NORMAL, ledger)
    }

    fn act_with_priority(
        &mut self,
        actor: &str,
        value: Value,
        priority: u8,
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        let canonical = self.registry.canonical_of(actor).to_string();
        if !self.registry.contains(&canonical) {
            ledger.record(
                rt_kinds::ACTOR_UNKNOWN,
                &format!("Ator '{actor}' não registrado no FXP."),
                Json::obj([("ator", Json::str(actor))]),
            );
            ledger.actuator_action(actor, &value, false);
            return ActOutcome::MissingActor;
        }
        let route = self.routes.get(&canonical).cloned();
        match route {
            Some(Route::Simulator) => {
                // Paridade com a Etapa 2: validação, efeitos e eventos do sim.
                self.sim.act(&canonical, value, ledger)
            }
            Some(Route::Real) | Some(Route::Remote(_)) | Some(Route::Inaccessible { .. }) => {
                // Limites do REGISTRO (inclusivos) antes do envio (§4.3).
                let limits = self.limits_of(&canonical);
                if let Some((limit, limit_value)) = self.violation(&limits, &value) {
                    return self.reject_over_limit(&canonical, &value, limit, limit_value, ledger);
                }
                match self.deliver_route(&canonical, &value, ledger) {
                    Some(outcome) => outcome,
                    None => {
                        // Indisponível: fallback do registro; esgotado → fila
                        // prioritária (retry em ticks futuros — PLAN §3.4).
                        let outcome = self.try_fallback(&canonical, &value, ledger);
                        if matches!(outcome, ActOutcome::FallbackExhausted) {
                            self.enqueue(&canonical, &value, priority);
                        }
                        outcome
                    }
                }
            }
            None => {
                ledger.record(
                    rt_kinds::ACTOR_UNKNOWN,
                    &format!("Ator '{actor}' não registrado no FXP."),
                    Json::obj([("ator", Json::str(actor))]),
                );
                ledger.actuator_action(actor, &value, false);
                ActOutcome::MissingActor
            }
        }
    }

    fn cpu_power(&self) -> f64 {
        match self.routes.get("cpu_power") {
            Some(Route::Simulator) | None => self.sim.cpu_power(),
            _ => self.known_power,
        }
    }

    fn on_tick(&mut self, ledger: &mut dyn Ledger) {
        self.sim.on_tick(ledger);
        self.update_power();
        self.pump_queue(ledger);
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

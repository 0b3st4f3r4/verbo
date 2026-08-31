//! Transporte do schema v1 sobre stream: Unix (local entre processos) e TCP
//! (remoto) — docs/FXP-SCHEMA-v1.md §7. Ack correlacionado por `seq` com
//! timeout de parede (§6); [`servir_unix`]/[`servir_tcp`] são o servidor de
//! referência (testes de integração e `fxpd` embutido).

use crate::schema::{self, Mensagem};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Falha de transporte (distingue timeout de schema — §4.1/§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroTransporte {
    ConexaoFalhou(String),
    Quebrada(String),
    /// Ack não chegou no prazo — vira falha de I/O no bus (§4.7), nunca dado.
    Timeout,
    Schema(schema::ErroSchema),
}

impl std::fmt::Display for ErroTransporte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErroTransporte::ConexaoFalhou(m) => write!(f, "conexão falhou: {m}"),
            ErroTransporte::Quebrada(m) => write!(f, "conexão quebrada: {m}"),
            ErroTransporte::Timeout => write!(f, "ack não chegou no prazo"),
            ErroTransporte::Schema(e) => write!(f, "violação do schema v1: {e}"),
        }
    }
}

impl std::error::Error for ErroTransporte {}

impl From<schema::ErroSchema> for ErroTransporte {
    fn from(e: schema::ErroSchema) -> Self {
        ErroTransporte::Schema(e)
    }
}

/// Fluxo bidirecional: Unix local ou TCP remoto — mesma semântica de frame.
enum Corrente {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Corrente {
    fn set_timeout(&self, d: Duration) {
        match self {
            Corrente::Unix(s) => {
                let _ = s.set_read_timeout(Some(d));
                let _ = s.set_write_timeout(Some(d));
            }
            Corrente::Tcp(s) => {
                let _ = s.set_read_timeout(Some(d));
                let _ = s.set_write_timeout(Some(d));
            }
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Corrente::Unix(s) => s.read(buf),
            Corrente::Tcp(s) => s.read(buf),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            Corrente::Unix(s) => s.write_all(buf),
            Corrente::Tcp(s) => s.write_all(buf),
        }
    }
}

/// Conexão cliente falando frames v1 (docs/FXP-SCHEMA-v1.md §2).
pub struct Conexao {
    corrente: Corrente,
}

impl Conexao {
    pub fn unix(path: &Path, timeout: Duration) -> Result<Self, ErroTransporte> {
        let s = UnixStream::connect(path)
            .map_err(|e| ErroTransporte::ConexaoFalhou(format!("{}: {e}", path.display())))?;
        s.set_nonblocking(false).ok();
        let c = Conexao { corrente: Corrente::Unix(s) };
        c.corrente.set_timeout(timeout);
        Ok(c)
    }

    pub fn tcp(host: &str, port: u16, timeout: Duration) -> Result<Self, ErroTransporte> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| ErroTransporte::ConexaoFalhou(format!("{host}:{port}: {e}")))?
            .next()
            .ok_or_else(|| ErroTransporte::ConexaoFalhou(format!("{host}:{port} sem endereço")))?;
        let s = TcpStream::connect(addr)
            .map_err(|e| ErroTransporte::ConexaoFalhou(format!("{addr}: {e}")))?;
        let c = Conexao { corrente: Corrente::Tcp(s) };
        c.corrente.set_timeout(timeout);
        Ok(c)
    }

    /// Envia a mensagem (frame completo).
    pub fn enviar(&mut self, msg: &Mensagem) -> Result<(), ErroTransporte> {
        let frame = schema::encode_to_vec(msg)?;
        self.corrente
            .write_all(&frame)
            .map_err(|e| ErroTransporte::Quebrada(format!("escrita: {e}")))
    }

    /// Recebe **um** frame respeitando o prazo total (`timeout` de parede).
    pub fn receber(&mut self, timeout: Duration) -> Result<Mensagem, ErroTransporte> {
        let prazo = Instant::now() + timeout;
        let mut ler = |buf: &mut [u8]| -> Result<usize, ErroTransporte> {
            loop {
                // Prazo recalculado a cada iteração: cada `read` bloqueia no
                // máximo o que falta; EAGAIN no fim do prazo → Timeout.
                let resta = prazo.saturating_duration_since(Instant::now());
                if resta.is_zero() {
                    return Err(ErroTransporte::Timeout);
                }
                self.corrente.set_timeout(resta);
                match self.corrente.read(buf) {
                    Ok(0) => return Err(ErroTransporte::Quebrada("fim do fluxo".into())),
                    Ok(n) => return Ok(n),
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::Interrupted =>
                    {
                        continue;
                    }
                    Err(e) => return Err(ErroTransporte::Quebrada(format!("leitura: {e}"))),
                }
            }
        };

        let mut prefixo = [0u8; 4];
        let mut lido = 0;
        while lido < 4 {
            lido += ler(&mut prefixo[lido..])?;
        }
        let total = schema::peek_frame_len(&prefixo)?;
        let mut frame = vec![0u8; total];
        frame[..4].copy_from_slice(&prefixo);
        let mut lido = 4;
        while lido < total {
            lido += ler(&mut frame[lido..])?;
        }
        let (msg, _) = schema::decode(&frame)?;
        Ok(msg)
    }

    /// Pedido-resposta: envia e espera o ack com o **mesmo seq** (§5).
    /// Resposta com seq divergente ⇒ conexão dessincronizada (erro, nunca
    /// ack trocado entre comandos).
    pub fn pedir(
        &mut self,
        msg: &Mensagem,
        timeout: Duration,
    ) -> Result<Mensagem, ErroTransporte> {
        self.enviar(msg)?;
        let resp = self.receber(timeout)?;
        if resp.seq != msg.seq {
            return Err(ErroTransporte::Quebrada(format!(
                "seq dessincronizado: enviado {}, recebido {}",
                msg.seq, resp.seq
            )));
        }
        Ok(resp)
    }
}

// ---------------------------------------------------------------------------
// Servidor de referência (testes/integração)
// ---------------------------------------------------------------------------

/// Handle do servidor ativo; `Drop` sinaliza o encerramento e agrega a thread.
pub struct Servidor {
    pub identificador: String,
    desligar: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Servidor {
    /// Sinaliza parada e espera a thread (o loop faz polling do flag).
    pub fn parar(mut self) {
        self.desligar.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Servidor {
    fn drop(&mut self) {
        self.desligar.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Servidor Unix: aceita conexões até `parar()`/`Drop`; **cada conexão é
/// servida em sua própria thread** (peers persistentes não bloqueiam novos
/// clientes). O `handler` devolve a resposta de cada pedido (`None` = sem
/// resposta — simula ator mudo para testar timeout).
pub fn servir_unix<F>(path: &Path, handler: F) -> Result<Servidor, ErroTransporte>
where
    F: Fn(Mensagem) -> Option<Mensagem> + Clone + Send + 'static,
{
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).map_err(|e| ErroTransporte::ConexaoFalhou(format!("{e}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| ErroTransporte::ConexaoFalhou(format!("{e}")))?;
    let desligar = Arc::new(AtomicBool::new(false));
    let flag = desligar.clone();
    let identificador = format!("unix:{}", path.display());
    let caminho = path.to_path_buf();
    let handle = std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((fluxo, _)) => {
                    let handler = handler.clone();
                    let flag_conn = flag.clone();
                    std::thread::spawn(move || {
                        servir_conexao(Corrente::Unix(fluxo), &handler, &flag_conn)
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
        let _ = std::fs::remove_file(&caminho);
    });
    Ok(Servidor { identificador, desligar, handle: Some(handle) })
}

/// Servidor TCP em porta efêmera; devolve a porta sorteada. Thread por
/// conexão, como no Unix.
pub fn servir_tcp<F>(handler: F) -> Result<(Servidor, u16), ErroTransporte>
where
    F: Fn(Mensagem) -> Option<Mensagem> + Clone + Send + 'static,
{
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| ErroTransporte::ConexaoFalhou(format!("{e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ErroTransporte::ConexaoFalhou(format!("{e}")))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| ErroTransporte::ConexaoFalhou(format!("{e}")))?;
    let desligar = Arc::new(AtomicBool::new(false));
    let flag = desligar.clone();
    let identificador = format!("tcp:127.0.0.1:{port}");
    let handle = std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((fluxo, _)) => {
                    let handler = handler.clone();
                    let flag_conn = flag.clone();
                    std::thread::spawn(move || {
                        servir_conexao(Corrente::Tcp(fluxo), &handler, &flag_conn)
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
    });
    Ok((Servidor { identificador, desligar, handle: Some(handle) }, port))
}

fn servir_conexao<F>(mut fluxo: Corrente, handler: &F, flag: &AtomicBool)
where
    F: Fn(Mensagem) -> Option<Mensagem>,
{
    fluxo.set_timeout(Duration::from_millis(250));
    let mut resto: Vec<u8> = Vec::new();
    loop {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        // Lê o próximo frame do fluxo (bloqueia até o timeout curto da conexão).
        match ler_frame(&mut fluxo, &mut resto) {
            Ok(Some(msg)) => {
                if msg.opcode == schema::op::BYE {
                    return;
                }
                if let Some(resp) = handler(msg) {
                    let frame = match schema::encode_to_vec(&resp) {
                        Ok(f) => f,
                        Err(_) => return,
                    };
                    if fluxo.write_all(&frame).is_err() {
                        return;
                    }
                }
            }
            Ok(None) => continue, // frame incompleto; lê mais
            Err(ErroTransporte::Timeout) => continue,
            Err(_) => return,
        }
    }
}

/// Extrai um frame de `resto` (buffer acumulado); devolve `None` se ainda
/// incompleto. Os bytes consumidos são removidos do buffer.
fn ler_frame(fluxo: &mut Corrente, resto: &mut Vec<u8>) -> Result<Option<Mensagem>, ErroTransporte> {
    if resto.len() >= 4 {
        let total = schema::peek_frame_len(resto)?;
        if resto.len() >= total {
            let (msg, _) = schema::decode(resto)?;
            resto.drain(..total);
            return Ok(Some(msg));
        }
    }
    let mut buf = [0u8; 4096];
    let n = match fluxo.read(&mut buf) {
        Ok(0) => return Err(ErroTransporte::Quebrada("fim do fluxo".into())),
        Ok(n) => n,
        Err(e)
            if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
        {
            return Err(ErroTransporte::Timeout)
        }
        Err(e) => return Err(ErroTransporte::Quebrada(format!("leitura: {e}"))),
    };
    resto.extend_from_slice(&buf[..n]);
    if resto.len() >= 4 {
        let total = schema::peek_frame_len(resto)?;
        if resto.len() >= total {
            let (msg, _) = schema::decode(resto)?;
            resto.drain(..total);
            return Ok(Some(msg));
        }
    }
    Ok(None)
}

/// Espera o servidor aceitar conexões (poll de conectividade).
pub fn esperar_pronto_unix(path: &Path, timeout: Duration) -> bool {
    let prazo = Instant::now() + timeout;
    while Instant::now() < prazo {
        if UnixStream::connect(path).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

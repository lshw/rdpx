use anyhow::{anyhow, bail, Context, Result};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use async_trait::async_trait;
use ironrdp_server::{
    CredentialDecision, CredentialValidationError, CredentialValidator, Credentials,
    DesktopSize, KeyboardEvent, MouseEvent, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpServerInputHandler,
};
use serde::Deserialize;
use std::{collections::HashMap, fs::File, io::BufReader, net::{IpAddr, SocketAddr},
    path::Path, sync::Arc, time::{Duration, Instant}};
use tokio::{io::copy_bidirectional, net::{TcpListener, TcpStream}, process::Command,
    sync::{Mutex, oneshot, Semaphore}, time::timeout};
use tokio_rustls::{rustls::{pki_types::{CertificateDer, PrivateKeyDer}, ServerConfig}, TlsAcceptor};
use tracing::{info, warn, error};

#[derive(Clone, Deserialize)]
struct Config {
    listen: SocketAddr,
    backend: SocketAddr,
    tls_cert: String,
    tls_key: String,
    username: String,
    password_hash_file: String,
    authorization_seconds: u64,
    max_clients: usize,
    handshake_timeout_seconds: u64,
    backend_connect_timeout_seconds: u64,
    #[serde(default)] on_authorized_script: Option<String>,
}

struct State {
    grants: Mutex<HashMap<IpAddr, Instant>>,
    /// One shared state for successful password checks. No credentials are stored.
    config: Config,
    password_hash: String,
}
impl State {
    async fn allowed(&self, ip: IpAddr) -> bool {
        let mut grants = self.grants.lock().await;
        grants.retain(|_, expires| *expires > Instant::now());
        grants.get(&ip).is_some_and(|expires| *expires > Instant::now())
    }
    async fn grant(&self, ip: IpAddr) -> Result<()> {
        if let Some(script) = &self.config.on_authorized_script {
            // Avoid shells. Run an absolute executable with one validated IP argument.
            if !Path::new(script).is_absolute() { bail!("authorization script path must be absolute"); }
            let status = timeout(Duration::from_secs(3), Command::new(script).arg(ip.to_string()).kill_on_drop(true).status()).await
                .context("authorization script timed out")??;
            if !status.success() { bail!("authorization script exited unsuccessfully"); }
        }
        self.grants.lock().await.insert(ip, Instant::now() + Duration::from_secs(self.config.authorization_seconds));
        Ok(())
    }
}

struct Validator {
    state: Arc<State>,
    ip: IpAddr,
    signal: Mutex<Option<oneshot::Sender<()>>>,
}
#[async_trait]
impl CredentialValidator for Validator {
    async fn validate(&self, creds: &Credentials) -> std::result::Result<CredentialDecision, CredentialValidationError> {
        // Argon2 verification is CPU intensive, so move to blocking pool.
        let name_ok = creds.username == self.state.config.username;
        let candidate = creds.password.clone();
        let hash = self.state.password_hash.clone();
        let good = tokio::task::spawn_blocking(move || {
            PasswordHash::new(&hash).ok().is_some_and(|parsed| Argon2::default().verify_password(candidate.as_bytes(), &parsed).is_ok())
        }).await.unwrap_or(false);
        if !name_ok || !good { warn!(peer = %self.ip, "invalid frontend credentials"); return Ok(CredentialDecision::Reject); }
        match self.state.grant(self.ip).await {
            Ok(()) => {
                info!(peer = %self.ip, "frontend authentication succeeded; next connection may proxy");
                if let Some(tx) = self.signal.lock().await.take() { let _ = tx.send(()); }
                Ok(CredentialDecision::Accept)
            }
            Err(e) => { error!(peer = %self.ip, error = %e, "authorization failed"); Ok(CredentialDecision::Reject) }
        }
    }
}

struct NoopInput;
impl RdpServerInputHandler for NoopInput {
    fn keyboard(&mut self, _: KeyboardEvent) {}
    fn mouse(&mut self, _: MouseEvent) {}
}

struct NoopDisplay;
#[async_trait]
impl RdpServerDisplay for NoopDisplay {
    async fn size(&mut self) -> DesktopSize { DesktopSize { width: 1024, height: 768 } }
    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        // No display is necessary: the connection terminates right after credential validation.
        bail!("authentication-only server does not support display")
    }
}

fn tls_acceptor(cert_path: &str, key_path: &str) -> Result<TlsAcceptor> {
    let mut cert_file = BufReader::new(File::open(cert_path)?);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_file).collect::<std::result::Result<_, _>>()?;
    let mut key_file = BufReader::new(File::open(key_path)?);
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key_file)?.context("missing private key")?;
    let cfg = ServerConfig::builder().with_no_client_auth().with_single_cert(certs, key)?;
    Ok(TlsAcceptor::from(Arc::new(cfg)))
}

async fn auth(stream: TcpStream, peer: SocketAddr, state: Arc<State>, acceptor: TlsAcceptor) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    let validator = Arc::new(Validator { state: state.clone(), ip: peer.ip(), signal: Mutex::new(Some(tx)) });
    let mut server = RdpServer::builder()
        .with_addr(state.config.listen)
        .with_tls(acceptor)
        .with_input_handler(NoopInput)
        .with_display_handler(NoopDisplay)
        .with_credential_validator(Some(validator))
        .build();
    let deadline = Duration::from_secs(state.config.handshake_timeout_seconds);
    tokio::select! {
        result = timeout(deadline, server.run_connection(stream)) => {
            match result { Ok(Ok(())) => Ok(()), Ok(Err(e)) => Err(e), Err(_) => bail!("frontend handshake timeout") }
        }
        _ = rx => {
            // Drop the server future and socket after successful TLS credential validation.
            // Client reconnects to the same port and enters TCP forwarding mode.
            Ok(())
        }
    }
}

async fn proxy(mut frontend: TcpStream, peer: SocketAddr, backend: SocketAddr, connect_timeout: Duration) -> Result<()> {
    let mut upstream = timeout(connect_timeout, TcpStream::connect(backend)).await
        .context("backend connection timeout")??;
    let (to_backend, to_client) = copy_bidirectional(&mut frontend, &mut upstream).await?;
    info!(%peer, %backend, to_backend, to_client, "RDP TCP proxy closed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    let path = std::env::args().nth(1).unwrap_or_else(|| "/etc/rdpx/config.toml".into());
    let config: Config = toml::from_str(&std::fs::read_to_string(path)?)?;
    if config.authorization_seconds == 0 || config.max_clients == 0 || config.listen == config.backend {
        bail!("invalid configuration: duration and concurrency must be positive; listen != backend");
    }
    let password_hash = std::fs::read_to_string(&config.password_hash_file)?.trim().to_string();
    PasswordHash::new(&password_hash).map_err(|e| anyhow!("invalid password hash: {e}"))?;
    let acceptor = tls_acceptor(&config.tls_cert, &config.tls_key)?;
    let listener = TcpListener::bind(config.listen).await?;
    let sem = Arc::new(Semaphore::new(config.max_clients));
    let state = Arc::new(State { grants: Mutex::new(HashMap::new()), config, password_hash });
    info!(listen = %state.config.listen, backend = %state.config.backend, "RDP gateway started");
    loop {
        let (stream, peer) = listener.accept().await?;
        let Ok(permit) = sem.clone().try_acquire_owned() else { warn!(%peer, "connection limit exceeded"); continue; };
        let state = state.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = if state.allowed(peer.ip()).await {
                proxy(stream, peer, state.config.backend,
                    Duration::from_secs(state.config.backend_connect_timeout_seconds)).await
            } else {
                auth(stream, peer, state, acceptor).await
            };
            if let Err(e) = result { warn!(%peer, error = %e, "connection ended"); }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn grant_expires() {
        let cfg = Config {
            listen: "127.0.0.1:3390".parse().unwrap(),
            backend: "127.0.0.1:3391".parse().unwrap(),
            tls_cert: String::new(), tls_key: String::new(),
            username: "test".into(), password_hash_file: String::new(),
            authorization_seconds: 1, max_clients: 10,
            handshake_timeout_seconds: 10, backend_connect_timeout_seconds: 3,
            on_authorized_script: None,
        };
        let state = State { grants: Mutex::new(HashMap::new()), config: cfg, password_hash: String::new() };
        let ip: IpAddr = "192.0.2.123".parse().unwrap();
        assert!(!state.allowed(ip).await);
        state.grant(ip).await.unwrap();
        assert!(state.allowed(ip).await);
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!state.allowed(ip).await);
    }
    #[tokio::test]
    async fn proxy_transports_unmodified_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend = backend_listener.local_addr().unwrap();
        let (frontend_listener, port) = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let p = l.local_addr().unwrap();
            (l, p)
        };
        let echo = tokio::spawn(async move {
            let (mut socket, _) = backend_listener.accept().await.unwrap();
            let mut buf = [0u8; 4];
            socket.read_exact(&mut buf).await.unwrap();
            socket.write_all(&buf).await.unwrap();
        });
        let gate = tokio::spawn(async move {
            let (stream, peer) = frontend_listener.accept().await.unwrap();
            proxy(stream, peer, backend, Duration::from_secs(2)).await.unwrap();
        });
        let mut c = TcpStream::connect(port).await.unwrap();
        c.write_all(b"RDP!").await.unwrap();
        let mut reply = [0u8; 4];
        c.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"RDP!");
        drop(c);
        echo.await.unwrap();
        gate.await.unwrap();
    }
}

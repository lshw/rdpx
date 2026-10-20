use anyhow::{bail, Context, Result};
use ironrdp_acceptor::{Acceptor, BeginResult};
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_server::{Credentials, DesktopSize, TlsIdentityCtx};
use ironrdp_tokio::TokioFramed;
use serde::Deserialize;
use std::{collections::HashMap, net::{IpAddr, SocketAddr},
    path::Path, sync::Arc, time::{Duration, Instant}};
use tokio::{io::copy_bidirectional, net::{TcpListener, TcpStream}, process::Command,
    sync::{Mutex, Semaphore}, time::timeout};
use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};
use tracing::{info, warn};

#[derive(Clone, Deserialize)]
struct Config {
    listen: SocketAddr,
    backend: SocketAddr,
    tls_cert: String,
    tls_key: String,
    username: String,
    password_file: String,
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
    password: String,
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

async fn auth(stream: TcpStream, peer: SocketAddr, state: Arc<State>, acceptor: TlsAcceptor, public_key: Arc<Vec<u8>>) -> Result<()> {
    let deadline = Duration::from_secs(state.config.handshake_timeout_seconds);
    timeout(deadline, async {
        let credentials = Credentials { username: state.config.username.clone(), password: state.password.clone(), domain: None };
        let mut rdp = Acceptor::new(SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX,
            DesktopSize { width: 1024, height: 768 }, Vec::new(), Some(credentials));
        let stream = match ironrdp_acceptor::accept_begin(TokioFramed::new(stream), &mut rdp).await.context("RDP negotiation failed")? {
            BeginResult::ShouldUpgrade(stream) => stream,
            BeginResult::Continue(_) => bail!("client did not negotiate NLA"),
        };
        let tls = acceptor.accept(stream).await.context("TLS handshake failed")?;
        let mut framed = TokioFramed::new(tls);
        rdp.mark_security_upgrade_as_done();
        if !rdp.should_perform_credssp() { bail!("client did not negotiate CredSSP"); }
        ironrdp_acceptor::accept_credssp(&mut framed, &mut rdp,
            &mut ironrdp_tokio::reqwest::ReqwestNetworkClient::new(),
            "rdp-client".into(), (*public_key).clone(), None).await.context("NLA authentication failed")?;
        state.grant(peer.ip()).await.context("authorization failed")?;
        info!(peer = %peer.ip(), "frontend NLA authentication succeeded; next connection may proxy");
        Ok(())
    }).await.context("frontend handshake timeout")?
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
    let password = std::fs::read_to_string(&config.password_file).context("reading NLA password file")?;
    // A single final line ending is allowed; other whitespace is part of the password.
    let password = password.strip_suffix("\r\n").or_else(|| password.strip_suffix('\n')).unwrap_or(&password).to_owned();
    if password.is_empty() || password.contains(['\r', '\n']) { bail!("NLA password file must contain one nonempty line"); }
    let identity = TlsIdentityCtx::init_from_paths(Path::new(&config.tls_cert), Path::new(&config.tls_key))?;
    let tls_config = ServerConfig::builder().with_no_client_auth()
        .with_single_cert(identity.certs.clone(), identity.priv_key.clone_key())?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let public_key = Arc::new(identity.pub_key);
    let listener = TcpListener::bind(config.listen).await?;
    let sem = Arc::new(Semaphore::new(config.max_clients));
    let state = Arc::new(State { grants: Mutex::new(HashMap::new()), config, password });
    info!(listen = %state.config.listen, backend = %state.config.backend, "RDP gateway started");
    tokio::task::LocalSet::new().run_until(async move {
        loop {
            let (stream, peer) = listener.accept().await?;
            let Ok(permit) = sem.clone().try_acquire_owned() else { warn!(%peer, "connection limit exceeded"); continue; };
            let state = state.clone();
            let acceptor = acceptor.clone();
            let public_key = public_key.clone();
            tokio::task::spawn_local(async move {
                let _permit = permit;
                let result = if state.allowed(peer.ip()).await {
                    proxy(stream, peer, state.config.backend,
                        Duration::from_secs(state.config.backend_connect_timeout_seconds)).await
                } else {
                    auth(stream, peer, state, acceptor, public_key).await
                };
                // Include the underlying protocol or I/O error, not just the
                // outer IronRDP context (for example, "accept_begin failed").
                if let Err(e) = result { warn!(%peer, error = %format_args!("{e:#}"), "connection ended"); }
            });
        }
    }).await
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
            username: "test".into(), password_file: String::new(),
            authorization_seconds: 1, max_clients: 10,
            handshake_timeout_seconds: 10, backend_connect_timeout_seconds: 3,
            on_authorized_script: None,
        };
        let state = State { grants: Mutex::new(HashMap::new()), config: cfg, password: String::new() };
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

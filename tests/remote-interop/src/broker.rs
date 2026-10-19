use std::{
    io::Write,
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// This process belongs only to this fixture. Never connects to a user's broker.
pub struct Broker {
    child: Child,
    _directory: tempfile::TempDir,
    pub nats_url: String,
    pub websocket_url: String,
}
impl Broker {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub async fn start() -> anyhow::Result<Self> {
        let tcp = TcpListener::bind(("127.0.0.1", 0))?;
        let websocket = TcpListener::bind(("127.0.0.1", 0))?;
        let port = tcp.local_addr()?.port();
        let ws_port = websocket.local_addr()?.port();
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("nats.conf");
        let mut file = std::fs::File::create(&config)?;
        writeln!(file, "host: 127.0.0.1\nport: {port}\nwebsocket {{ host: 127.0.0.1, port: {ws_port}, no_tls: true }}")?;
        drop(file);
        drop((tcp, websocket));
        let child = Command::new("nats-server")
            .arg("-c")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut broker = Self {
            child,
            _directory: directory,
            nats_url: format!("nats://127.0.0.1:{port}"),
            websocket_url: format!("ws://127.0.0.1:{ws_port}"),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = broker.child.try_wait()? {
                anyhow::bail!("fixture broker exited: {status}");
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
                && tokio::net::TcpStream::connect(("127.0.0.1", ws_port))
                    .await
                    .is_ok()
            {
                return Ok(broker);
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "fixture broker startup timed out"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

//! Private JSON-lines IPC for the TypeScript driver. Stdout must not be logged.
use future_remote_interop::{Broker, Fixture};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

async fn send(value: Value) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    tokio::io::stdout().write_all(&bytes).await?;
    tokio::io::stdout().flush().await?;
    Ok(())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let broker = Broker::start().await?;
    let fixture = Fixture::start(&broker.nats_url, &broker.websocket_url).await?;
    send(json!({"type":"ready", "brokerPid": broker.pid(), "credentials": fixture.client_credentials, "desktopNkey": fixture.desktop_nkey})).await?;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let command: Value = serde_json::from_str(&line)?;
        let id = &command["id"];
        match command["type"].as_str() {
            Some("emit") => {
                fixture.emit()?;
                send(json!({"id": id, "ok": true})).await?;
            }
            Some("metrics") => {
                send(json!({"id": id, "metrics": fixture.metrics()})).await?;
            }
            Some("shutdown") => {
                fixture.shutdown().await?;
                send(json!({"id": id, "ok": true})).await?;
                return Ok(());
            }
            _ => anyhow::bail!("unknown harness control operation"),
        }
    }
    fixture.shutdown().await?;
    Ok(())
}

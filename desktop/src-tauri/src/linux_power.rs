//! systemd-logind sleep/shutdown notifications for Linux desktops.

use futures::StreamExt;
use std::time::Duration;

pub fn install_disconnect_notifier() {
    tauri::async_runtime::spawn(async {
        loop {
            if let Err(error) = listen().await {
                eprintln!("remote: Linux power notification listener unavailable: {error}");
            }
            // login1 is absent on non-systemd systems and can restart during an
            // upgrade. Retry slowly; heartbeat expiry remains the fallback.
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

async fn listen() -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let mut sleep = proxy.receive_signal("PrepareForSleep").await?;
    let mut shutdown = proxy.receive_signal("PrepareForShutdown").await?;

    loop {
        tokio::select! {
            message = sleep.next() => {
                let Some(message) = message else { return Ok(()); };
                if message.body().deserialize::<bool>().unwrap_or(false) {
                    crate::remote::handle_system_suspend().await;
                } else {
                    crate::remote::handle_system_resume();
                }
            }
            message = shutdown.next() => {
                let Some(message) = message else { return Ok(()); };
                if message.body().deserialize::<bool>().unwrap_or(false) {
                    crate::remote::notify_mobile_disconnect("system_power_off").await;
                }
            }
        }
    }
}

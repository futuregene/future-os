//! Desktop environment adapter for the shared controller connection.
pub(crate) use future_remote_client::session::*;

pub(crate) async fn connect(creds: &super::creds::PeerCreds) -> Result<Connected, crate::Error> {
    let environment = connection_options();
    future_remote_client::session::connect(creds, &environment)
        .await
        .map_err(Into::into)
}

fn connection_options() -> ConnectionOptions {
    let mut environment = ConnectionOptions::new(super::runtime::device_name(), "desktop");
    environment.root_bundle = crate::remote::trust::root_bundle();
    #[cfg(test)]
    let environment = environment.for_loopback_fixture();
    environment
}

#[cfg(test)]
pub(crate) async fn exchange_for_test(
    client: &async_nats::Client,
    subject: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, crate::Error> {
    future_remote_client::session::exchange_for_test(client, subject, payload)
        .await
        .map_err(Into::into)
}

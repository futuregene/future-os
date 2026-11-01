use future_remote_client::{
    credentials::{PeerBook, PeerCreds},
    host::{Claimed, ClientHost, Refreshed},
    ConnectionOptions, Error,
};
use future_remote_protocol::invitation::Invitation;
use futures::{future::BoxFuture, FutureExt};

pub(super) struct DesktopClientHost;
fn convert(error: crate::Error) -> Error {
    match error {
        crate::Error::Io(error) => Error::Io(error),
        crate::Error::Json(error) => Error::Json(error),
        crate::Error::Remote {
            status,
            code,
            message,
        } => Error::Remote {
            status,
            code,
            message,
        },
        crate::Error::RemoteTransport(message) => Error::RemoteTransport(message),
        crate::Error::RemoteAuthorization(message) => Error::RemoteAuthorization(message),
        crate::Error::Message(message) => Error::Message(message),
        error => Error::Host(Box::new(error)),
    }
}
impl ClientHost for DesktopClientHost {
    fn device_id(&self) -> Result<String, Error> {
        crate::device_identity::device_id().map_err(convert)
    }
    fn connection_options(&self) -> ConnectionOptions {
        let mut options = ConnectionOptions::new(super::runtime::device_name(), "desktop");
        options.root_bundle = crate::remote::trust::root_bundle();
        #[cfg(any(test, feature = "test-support"))]
        let options = if crate::remote::fixture_mode() {
            options.for_loopback_fixture()
        } else {
            options
        };
        options
    }
    fn command_id(&self) -> String {
        crate::store::create_id("cmd")
    }
    fn load(&self) -> Result<PeerBook, Error> {
        super::creds::load().map_err(convert)
    }
    fn upsert(&self, creds: PeerCreds) -> Result<(), Error> {
        super::creds::upsert(creds).map_err(convert)
    }
    fn update_credentials(&self, expected: &PeerCreds, fresh: &Refreshed) -> Result<bool, Error> {
        super::creds::update_credentials_if_current(expected, fresh).map_err(convert)
    }
    fn clear_secret(&self, expected: &PeerCreds) -> Result<bool, Error> {
        super::creds::clear_secret_if_current(expected).map_err(convert)
    }
    fn remove(&self, expected: &PeerCreds) -> Result<bool, Error> {
        super::creds::remove_if_current(expected).map_err(convert)
    }
    fn set_label(
        &self,
        desktop: &str,
        name: Option<&str>,
        icon: Option<&str>,
    ) -> Result<(), Error> {
        super::creds::set_label(desktop, name, icon).map_err(convert)
    }
    fn claim<'a>(
        &'a self,
        invite: &'a Invitation,
        device: &'a str,
        key: &'a str,
        name: &'a str,
    ) -> BoxFuture<'a, Result<Claimed, Error>> {
        async move {
            super::platform::claim(invite, device, key, name)
                .await
                .map_err(convert)
        }
        .boxed()
    }
    fn refresh<'a>(&'a self, creds: &'a PeerCreds) -> BoxFuture<'a, Result<Refreshed, Error>> {
        async move { super::platform::refresh(creds).await.map_err(convert) }.boxed()
    }
    fn revoke<'a>(&'a self, creds: &'a PeerCreds) -> BoxFuture<'a, Result<(), Error>> {
        async move { super::platform::revoke(creds).await.map_err(convert) }.boxed()
    }
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        drop(crate::runtime::spawn(future));
    }
}

#[cfg(test)]
mod tests {
    use super::convert;
    use future_remote_client::Error as ClientError;

    /// Every remote-client call reports failure through this one conversion, so
    /// the variants that carry meaning across the boundary have to survive it:
    /// `Remote` keeps the HTTP status and machine-readable code the pairing UI
    /// branches on, and a transport or authorization failure must not arrive as
    /// a generic host error. Anything the client's vocabulary has no slot for is
    /// wrapped with the original still downcastable, so a signed-out desktop is
    /// not flattened into a message string.
    #[test]
    fn a_host_error_keeps_its_variant_across_the_client_boundary() {
        let malformed = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        let json = convert(crate::Error::Json(malformed));
        assert!(matches!(json, ClientError::Json(_)), "{json:?}");

        let transport = convert(crate::Error::RemoteTransport("relay unreachable".into()));
        assert!(
            matches!(transport, ClientError::RemoteTransport(ref message)
                if message == "relay unreachable"),
            "{transport:?}"
        );

        let authorization = convert(crate::Error::RemoteAuthorization("rejected".into()));
        assert!(
            matches!(authorization, ClientError::RemoteAuthorization(ref message)
                if message == "rejected"),
            "{authorization:?}"
        );

        let remote = convert(crate::Error::Remote {
            status: 403,
            code: Some("invalid_remote_credential".into()),
            message: "denied".into(),
        });
        assert!(
            matches!(
                remote,
                ClientError::Remote {
                    status: 403,
                    ref code,
                    ref message,
                } if code.as_deref() == Some("invalid_remote_credential") && message == "denied"
            ),
            "{remote:?}"
        );

        let message = convert(crate::Error::Message("a plain failure".into()));
        assert!(
            matches!(message, ClientError::Message(ref text) if text == "a plain failure"),
            "{message:?}"
        );

        let unsigned = convert(crate::Error::NotSignedIn);
        match unsigned {
            ClientError::Host(error) => {
                let original = error
                    .downcast::<crate::Error>()
                    .expect("a host-only error keeps its type");
                assert!(matches!(*original, crate::Error::NotSignedIn));
            }
            other => panic!("a host-only error must stay typed, got {other:?}"),
        }
    }
}

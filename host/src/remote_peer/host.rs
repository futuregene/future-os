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

//! Unified application error.
//!
//! Wraps the common fallible sources (SQLite, IO) so `?` converts them
//! automatically — replacing the previous `.map_err(|e| e.to_string())` noise —
//! and carries free-form messages for everything else (gRPC/transport errors,
//! validation, not-found lookups). It serializes to a plain string, so the
//! Tauri command error contract seen by the frontend (`Result<T, string>`)
//! is unchanged.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The Future Agent gRPC endpoint was unreachable. A distinct variant (not
    /// `Message`) so callers like `abort_run` can tolerate a down agent by
    /// matching the type instead of sniffing the error string. The message is
    /// carried verbatim, so the serialized string the frontend sees is unchanged.
    #[error("{0}")]
    AgentUnavailable(String),
    #[error("{message}")]
    AgentCommand { code: String, message: String },
    #[error("{reason}")]
    AgentIncompatible {
        agent_version: Option<String>,
        reason: String,
    },
    /// A remote-control platform call failed. Carries the HTTP status and the
    /// server's machine-readable error code (the `error` field of the JSON
    /// error body, e.g. `invalid_remote_credential`) so callers can react by
    /// matching the code instead of sniffing the human-readable message. The
    /// display string stays the plain message, so the serialized contract the
    /// frontend sees is unchanged.
    #[error("{message}")]
    Remote {
        status: u16,
        code: Option<String>,
        message: String,
    },
    /// A remote-control platform call never reached a response (DNS failure,
    /// refused connection, timeout, offline). Distinct from [`Error::Remote`]
    /// — which carries an HTTP status — so the UI can show a "check your
    /// network" message instead of a raw transport error. The display string
    /// stays the plain message, so the serialized contract the frontend sees is
    /// unchanged.
    #[error("{0}")]
    RemoteTransport(String),
    /// NATS accepted the transport but rejected the freshly issued bridge
    /// credential. This is a deployment/account authorization failure, not a
    /// network outage, so retrying forever cannot repair it.
    #[error("{0}")]
    RemoteAuthorization(String),
    /// This desktop holds no FutureOS account credential. Typed (not a
    /// `Message`) because the Remote page has to tell "sign in again" apart
    /// from "this device is broken": pairing codes are issued by the platform,
    /// so a signed-out desktop can never start the bridge, and that is an
    /// account fault the user can fix rather than a local one to retry.
    #[error("Not signed in to FutureOS.")]
    NotSignedIn,
    /// Another process holds this data root's Agent instance lock. Typed so the
    /// supervision loop can treat it as "yield and wait" instead of a fault:
    /// the holder is starting or serving the Agent, and starting a second one
    /// is exactly what the lock forbids.
    #[error("{0}")]
    AgentBusy(String),
    #[error("{0}")]
    Message(String),
}

impl From<future_remote_server::Error> for Error {
    fn from(error: future_remote_server::Error) -> Self {
        use future_remote_server::Error as ServerError;
        match error {
            ServerError::Host(error) | ServerError::AccountAuthorization(error) => {
                match error.downcast::<Error>() {
                    Ok(error) => *error,
                    Err(error) => Self::Message(error.to_string()),
                }
            }
            ServerError::Io(error) => Self::Io(error),
            ServerError::Json(error) => Self::Json(error),
            ServerError::Remote {
                status,
                code,
                message,
            } => Self::Remote {
                status,
                code,
                message,
            },
            ServerError::RemoteTransport(message) => Self::RemoteTransport(message),
            ServerError::RemoteAuthorization(message) => Self::RemoteAuthorization(message),
            ServerError::Message(message) => Self::Message(message),
        }
    }
}
impl From<Error> for future_remote_server::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::Io(error) => Self::Io(error),
            Error::Json(error) => Self::Json(error),
            Error::Remote {
                status,
                code,
                message,
            } => Self::Remote {
                status,
                code,
                message,
            },
            Error::RemoteTransport(message) => Self::RemoteTransport(message),
            Error::RemoteAuthorization(message) => Self::RemoteAuthorization(message),
            Error::NotSignedIn => Self::AccountAuthorization(Box::new(Error::NotSignedIn)),
            Error::Message(message) => Self::Message(message),
            error => Self::Host(Box::new(error)),
        }
    }
}

impl From<future_remote_client::Error> for Error {
    fn from(error: future_remote_client::Error) -> Self {
        match error {
            future_remote_client::Error::Host(error) => match error.downcast::<Error>() {
                Ok(error) => *error,
                Err(error) => Self::Message(error.to_string()),
            },
            future_remote_client::Error::Io(error) => Self::Io(error),
            future_remote_client::Error::Json(error) => Self::Json(error),
            future_remote_client::Error::Remote {
                status,
                code,
                message,
            } => Self::Remote {
                status,
                code,
                message,
            },
            future_remote_client::Error::RemoteTransport(message) => Self::RemoteTransport(message),
            future_remote_client::Error::RemoteAuthorization(message) => {
                Self::RemoteAuthorization(message)
            }
            future_remote_client::Error::Message(message) => Self::Message(message),
        }
    }
}

impl From<future_app_settings::Error> for Error {
    fn from(error: future_app_settings::Error) -> Self {
        match error {
            future_app_settings::Error::Database(error) => Error::Database(error),
            future_app_settings::Error::Json(error) => Error::Json(error),
            future_app_settings::Error::Io(error) => Error::Io(error),
            future_app_settings::Error::Invalid(message) => Error::Message(message),
        }
    }
}

impl From<future_app_workspaces::Error> for Error {
    fn from(error: future_app_workspaces::Error) -> Self {
        match error {
            future_app_workspaces::Error::Database(error) => Error::Database(error),
            future_app_workspaces::Error::Io(error) => Error::Io(error),
            future_app_workspaces::Error::Invalid(message) => Error::Message(message),
        }
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Error::Message(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Error::Message(message.to_string())
    }
}

impl serde::Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_boundary_preserves_recovery_categories_and_original_host_errors() {
        use future_remote_server::Error as ServerError;
        let platform = Error::Remote {
            status: 403,
            code: Some("invalid_remote_credential".into()),
            message: "revoked fixture".into(),
        };
        let server: ServerError = platform.into();
        assert!(
            matches!(&server, ServerError::Remote { status: 403, code: Some(code), .. } if code == "invalid_remote_credential")
        );
        let platform: Error = server.into();
        assert!(
            matches!(&platform, Error::Remote { status: 403, code: Some(code), .. } if code == "invalid_remote_credential")
        );
        assert_eq!(platform.to_string(), "revoked fixture");
        let account: ServerError = Error::NotSignedIn.into();
        assert!(matches!(&account, ServerError::AccountAuthorization(_)));
        assert!(matches!(Error::from(account), Error::NotSignedIn));
        let transport: ServerError = Error::RemoteTransport("offline fixture".into()).into();
        assert!(matches!(&transport, ServerError::RemoteTransport(_)));
        assert!(matches!(Error::from(transport), Error::RemoteTransport(_)));
        let authorization: ServerError = Error::RemoteAuthorization("grant fixture".into()).into();
        assert!(matches!(
            &authorization,
            ServerError::RemoteAuthorization(_)
        ));
        assert!(matches!(
            Error::from(authorization),
            Error::RemoteAuthorization(_)
        ));
        let business: ServerError = Error::AgentCommand {
            code: "fixture_code".into(),
            message: "fixture message".into(),
        }
        .into();
        assert!(matches!(&business, ServerError::Host(_)));
        assert!(
            matches!(Error::from(business), Error::AgentCommand { code, message } if code == "fixture_code" && message == "fixture message")
        );
    }

    #[test]
    fn controller_error_conversion_preserves_retry_categories_and_wire_codes() {
        let transport: Error =
            future_remote_client::Error::RemoteTransport("network fixture".into()).into();
        assert!(matches!(&transport, Error::RemoteTransport(_)));
        assert_eq!(transport.to_string(), "network fixture");
        let authorization: Error =
            future_remote_client::Error::RemoteAuthorization("credential fixture".into()).into();
        assert!(matches!(&authorization, Error::RemoteAuthorization(_)));
        assert_eq!(authorization.to_string(), "credential fixture");
        let platform: Error = future_remote_client::Error::Remote {
            status: 401,
            code: Some("invalid_remote_credential".into()),
            message: "revoked fixture".into(),
        }
        .into();
        assert!(matches!(
            &platform,
            Error::Remote { status: 401, code: Some(code), .. }
                if code == "invalid_remote_credential"
        ));
        assert_eq!(
            serde_json::to_string(&platform).unwrap(),
            "\"revoked fixture\""
        );
    }

    #[test]
    fn serializes_as_plain_string() {
        let serialized = serde_json::to_string(&Error::Message("boom".to_string())).unwrap();
        assert_eq!(serialized, "\"boom\"");

        let from_string: Error = "plain".to_string().into();
        assert_eq!(serde_json::to_string(&from_string).unwrap(), "\"plain\"");

        let remote = Error::Remote {
            status: 502,
            code: Some("bad_gateway".to_string()),
            message: "upstream failed".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&remote).unwrap(),
            "\"upstream failed\""
        );
    }

    /// `app-settings` / `app-workspaces` surface their own error type; the
    /// conversions here are what keep an IO/JSON failure from being flattened
    /// into a `Message` (callers match on `Error::Io`/`Json`). Each source
    /// variant is mapped by hand, so each arm needs its own conversion.
    #[test]
    fn converts_shared_crate_errors_into_typed_app_errors() {
        let json = serde_json::from_str::<u8>("not json").unwrap_err();
        let settings_json: Error = future_app_settings::Error::Json(json).into();
        assert!(matches!(settings_json, Error::Json(_)));

        let settings_io: Error =
            future_app_settings::Error::Io(std::io::Error::other("settings io")).into();
        assert!(matches!(settings_io, Error::Io(_)));
        assert!(settings_io.to_string().contains("settings io"));

        let workspaces_io: Error =
            future_app_workspaces::Error::Io(std::io::Error::other("workspaces io")).into();
        assert!(matches!(workspaces_io, Error::Io(_)));
        assert!(workspaces_io.to_string().contains("workspaces io"));
    }

    /// The relay and remote-client layers wrap a host error in a boxed `dyn
    /// Error`; the conversion is what puts it back. A payload that is ours keeps
    /// its variant (so a caller matching `NotSignedIn` still sees it), and one
    /// that is not ours becomes a message instead of being lost.
    #[test]
    fn a_boxed_remote_error_is_unwrapped_only_when_it_is_ours() {
        use future_remote_client::Error as ClientError;
        use future_remote_server::Error as ServerError;

        // server → host: a host payload survives, a foreign one becomes a message.
        let ours: Error = ServerError::Host(Box::new(Error::NotSignedIn)).into();
        assert!(matches!(ours, Error::NotSignedIn), "{ours:?}");
        let foreign: Error = ServerError::Host(Box::new(std::io::Error::other("not ours"))).into();
        assert!(
            matches!(&foreign, Error::Message(text) if text.contains("not ours")),
            "{foreign:?}"
        );
        let account: Error =
            ServerError::AccountAuthorization(Box::new(std::io::Error::other("signed out"))).into();
        assert!(
            matches!(&account, Error::Message(text) if text.contains("signed out")),
            "{account:?}"
        );

        // client → host: the same unwrapping, plus the JSON kind kept as itself
        // so a settings writer can still tell "corrupt file" from "transport".
        let ours: Error = ClientError::Host(Box::new(Error::NotSignedIn)).into();
        assert!(matches!(ours, Error::NotSignedIn), "{ours:?}");
        let foreign: Error = ClientError::Host(Box::new(std::io::Error::other("not ours"))).into();
        assert!(
            matches!(&foreign, Error::Message(text) if text.contains("not ours")),
            "{foreign:?}"
        );

        let invalid = serde_json::from_str::<i32>("not json").expect_err("invalid json");
        let server_json: Error = ServerError::Json(invalid).into();
        assert!(matches!(server_json, Error::Json(_)), "{server_json:?}");
        let invalid = serde_json::from_str::<i32>("not json").expect_err("invalid json");
        let client_json: Error = ClientError::Json(invalid).into();
        assert!(matches!(client_json, Error::Json(_)), "{client_json:?}");
    }
}

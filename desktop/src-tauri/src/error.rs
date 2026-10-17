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
pub enum AppError {
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
    /// refused connection, timeout, offline). Distinct from [`AppError::Remote`]
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
    #[error("{0}")]
    Message(String),
}

impl From<future_app_settings::Error> for AppError {
    fn from(error: future_app_settings::Error) -> Self {
        match error {
            future_app_settings::Error::Database(error) => AppError::Database(error),
            future_app_settings::Error::Json(error) => AppError::Json(error),
            future_app_settings::Error::Io(error) => AppError::Io(error),
            future_app_settings::Error::Invalid(message) => AppError::Message(message),
        }
    }
}

impl From<future_app_workspaces::Error> for AppError {
    fn from(error: future_app_workspaces::Error) -> Self {
        match error {
            future_app_workspaces::Error::Database(error) => AppError::Database(error),
            future_app_workspaces::Error::Io(error) => AppError::Io(error),
            future_app_workspaces::Error::Invalid(message) => AppError::Message(message),
        }
    }
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        AppError::Message(message)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        AppError::Message(message.to_string())
    }
}

impl serde::Serialize for AppError {
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
    fn serializes_as_plain_string() {
        let serialized = serde_json::to_string(&AppError::Message("boom".to_string())).unwrap();
        assert_eq!(serialized, "\"boom\"");

        let from_string: AppError = "plain".to_string().into();
        assert_eq!(serde_json::to_string(&from_string).unwrap(), "\"plain\"");

        let remote = AppError::Remote {
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
    /// into a `Message` (callers match on `AppError::Io`/`Json`). Each source
    /// variant is mapped by hand, so each arm needs its own conversion.
    #[test]
    fn converts_shared_crate_errors_into_typed_app_errors() {
        let json = serde_json::from_str::<u8>("not json").unwrap_err();
        let settings_json: AppError = future_app_settings::Error::Json(json).into();
        assert!(matches!(settings_json, AppError::Json(_)));

        let settings_io: AppError =
            future_app_settings::Error::Io(std::io::Error::other("settings io")).into();
        assert!(matches!(settings_io, AppError::Io(_)));
        assert!(settings_io.to_string().contains("settings io"));

        let workspaces_io: AppError =
            future_app_workspaces::Error::Io(std::io::Error::other("workspaces io")).into();
        assert!(matches!(workspaces_io, AppError::Io(_)));
        assert!(workspaces_io.to_string().contains("workspaces io"));
    }
}

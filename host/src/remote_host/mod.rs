//! Desktop integration for the embedded remote controller, shared by the GUI
//! and the explicitly started foreground headless mode.
pub(crate) mod availability;
pub(crate) mod business;
pub(crate) mod catalog;
mod command_diagnostics;
pub(crate) mod files;
pub(crate) mod lean;
pub(crate) mod pairing;
mod read_pages;
mod session_files;
#[cfg(test)]
mod sync_measurement;
use future_remote_server::services::{BusinessHost, ReplySink};
struct DesktopHost;
impl BusinessHost for DesktopHost {
    fn execute<'a>(
        &'a self,
        command: future_remote_protocol::IncomingCmd,
        reply: &'a dyn ReplySink,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            // Measure the desktop's half of every remote command, so a phone's
            // load timeout can be told apart from a slow relay: no slow row
            // logged here means the desktop answered quickly.
            let kind = command.cmd_type.clone();
            let session = command.session_id.clone();
            let started = std::time::Instant::now();
            let outcome = std::sync::Mutex::new(command_diagnostics::CommandOutcome::default());
            let sink = command_diagnostics::ObservedSink::new(reply, &outcome);
            if command.cmd_type == "get_read_chunk" {
                match read_pages::read(&command) {
                    Ok(page) => sink.send_binary(page.header, page.body).await,
                    Err(error) => sink.send(false, Value::Null, Some(error)).await,
                }
                command_diagnostics::report(&kind, &session, started.elapsed(), &outcome);
                return;
            }
            let paged = read_pages::PagedReply::new(&command, &sink);
            business::execute(command, &paged).await;
            command_diagnostics::report(&kind, &session, started.elapsed(), &outcome);
        })
    }
}
pub(crate) fn host() -> &'static dyn RemoteHost {
    &DesktopHost
}

pub(crate) fn owned_host() -> std::sync::Arc<dyn RemoteHost> {
    std::sync::Arc::new(DesktopHost)
}

pub(crate) fn attach_events() {
    crate::agent_events::attach(|event| {
        crate::remote::publish_event(
            event.session_id,
            event.event_type,
            event.data,
            event.run_id,
            event.idx,
            event.epoch,
            event.event_id,
            event.timestamp,
            event.session_idx,
            event.run_sequence,
        )
    });
}
use future_remote_server::services::{
    FileHost, Invitation, PairingHost, RemoteHost, SessionsCatalog, StateHost,
};
use future_remote_server::PairingCreds;
use futures::future::BoxFuture;
use serde_json::Value;
impl PairingHost for DesktopHost {
    fn load(&self) -> Option<PairingCreds> {
        pairing::load_creds()
    }
    fn load_slots(&self) -> Vec<PairingCreds> {
        pairing::load_book()
            .map(|book| book.slots().to_vec())
            .unwrap_or_default()
    }
    fn active_slot(&self) -> Option<String> {
        pairing::load_book().and_then(|book| book.active().map(str::to_string))
    }
    fn set_active(&self, pair_id: &str) -> Result<(), future_remote_server::Error> {
        pairing::set_active(pair_id).map(|_| ()).map_err(Into::into)
    }
    fn remove_slot(&self, pair_id: &str) -> Result<(), future_remote_server::Error> {
        pairing::remove_slot(pair_id).map_err(Into::into)
    }
    fn save(&self, c: &PairingCreds) -> Result<(), future_remote_server::Error> {
        pairing::save_creds(c).map_err(Into::into)
    }
    fn clear(&self) -> Result<(), future_remote_server::Error> {
        pairing::clear_creds().map_err(Into::into)
    }
    fn invite(
        &self,
        keep_existing: bool,
        shown: &[String],
    ) -> BoxFuture<'_, Result<Invitation, future_remote_server::Error>> {
        // Copied because the future outlives a borrow of the caller's slice,
        // which the trait's single elided lifetime cannot tie to `self`.
        let shown = shown.to_vec();
        Box::pin(async move {
            pairing::create_pairing(keep_existing, &shown)
                .await
                .map_err(Into::into)
        })
    }
    fn refresh(
        &self,
        c: PairingCreds,
    ) -> BoxFuture<'_, Result<PairingCreds, future_remote_server::Error>> {
        Box::pin(async move { pairing::refresh_bridge_jwt(c).await.map_err(Into::into) })
    }
    fn queue_revoke(&self, c: &PairingCreds) -> Result<(), future_remote_server::Error> {
        pairing::queue_revoke(c).map_err(Into::into)
    }
    fn queue_revoke_invitation(&self, pair_id: &str) -> Result<(), future_remote_server::Error> {
        pairing::queue_revoke_invitation(pair_id).map_err(Into::into)
    }
    fn retry_revokes(&self) -> BoxFuture<'_, Result<(), future_remote_server::Error>> {
        Box::pin(async { pairing::retry_pending_revokes().await.map_err(Into::into) })
    }
}
impl StateHost for DesktopHost {
    fn catalog_epoch(&self) -> String {
        catalog::epoch()
    }
    fn sessions(&self, pair: &str) -> Option<(Value, String)> {
        catalog::sessions(pair)
    }
    fn sessions_catalog(&self, pair: &str) -> Option<SessionsCatalog> {
        catalog::sessions_catalog(pair)
    }
    fn workspaces(&self) -> Option<(Value, String)> {
        catalog::workspaces()
    }
    fn catalog_revisions(&self) -> (String, u64, u64) {
        catalog::revisions()
    }
    fn catalog_dirty(&self) -> bool {
        crate::store::take_catalog_dirty()
    }
    fn agent_available(&self) -> bool {
        availability::available()
    }
    fn monitor_agent(&self) -> BoxFuture<'_, ()> {
        Box::pin(availability::monitor())
    }
    fn on_control_claimed(&self) {
        // The windowed desktop activates its main window so the person who
        // took control sees the machine they now own. Headless hosts have no
        // presentation sink and this is a no-op there.
        crate::emit_control_claimed();
    }
}
impl FileHost for DesktopHost {
    fn upload_chunk(&self, id: &str, index: u64, bytes: &[u8]) -> Result<Value, String> {
        files::write_upload_chunk(id, index, bytes)
    }
    fn download_chunk(&self, id: &str, index: u64) -> Result<Vec<u8>, future_remote_server::Error> {
        files::read_download_chunk(id, index).map_err(Into::into)
    }
    fn prune_transfers(&self) {
        files::prune_expired();
        read_pages::prune();
    }
    fn clear_transfers(&self) {
        files::clear_transfers();
        read_pages::clear();
    }
    fn clear_preview_cache(&self) {
        files::clear_preview_cache();
    }
}

#[cfg(test)]
mod tests {
    use future_remote_server::services::StateHost;

    /// Taking control raises the GUI's claim hook. A headless host has no
    /// presentation, so the hook must return without installing one: the
    /// window activation belongs to the windowed entry point, and a backend
    /// that attached a sink here would make every later `publish_to_gui` in
    /// this process try to reach it. `events::attach` is process-global and
    /// one-shot and another test may already have attached one, so what the
    /// hook is held to is that it never changes whether one exists.
    #[test]
    fn the_claim_hook_does_not_attach_a_presentation() {
        let attached = crate::events::has_presentation();
        StateHost::on_control_claimed(&super::DesktopHost);
        assert_eq!(
            crate::events::has_presentation(),
            attached,
            "the backend hook must not install a presentation sink"
        );
    }
}

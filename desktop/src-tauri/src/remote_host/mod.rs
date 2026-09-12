//! Desktop integration for the embedded remote controller; never a background service.
pub(crate) mod availability;
pub(crate) mod business;
pub(crate) mod catalog;
pub(crate) mod files;
pub(crate) mod pairing;
mod read_pages;
use crate::remote::services::{BusinessHost, ReplySink};
struct DesktopHost;
impl BusinessHost for DesktopHost {
    fn execute<'a>(
        &'a self,
        command: crate::remote::protocol::IncomingCmd,
        reply: &'a dyn ReplySink,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            if command.cmd_type == "get_read_chunk" {
                match read_pages::read(&command) {
                    Ok(data) => reply.send(true, data, None).await,
                    Err(error) => reply.send(false, Value::Null, Some(error)).await,
                }
                return;
            }
            let paged = read_pages::PagedReply::new(&command, reply);
            business::execute(command, &paged).await;
        })
    }
}
pub(crate) fn host() -> &'static dyn RemoteHost {
    &DesktopHost
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
use crate::remote::protocol::PairingCreds;
use crate::remote::services::{FileHost, Invitation, PairingHost, RemoteHost, StateHost};
use futures::future::BoxFuture;
use serde_json::Value;
impl PairingHost for DesktopHost {
    fn load(&self) -> Option<PairingCreds> {
        pairing::load_creds()
    }
    fn save(&self, c: &PairingCreds) -> Result<(), crate::AppError> {
        pairing::save_creds(c)
    }
    fn clear(&self) -> Result<(), crate::AppError> {
        pairing::clear_creds()
    }
    fn invite(&self) -> BoxFuture<'_, Result<Invitation, crate::AppError>> {
        Box::pin(pairing::create_pairing())
    }
    fn refresh(&self, c: PairingCreds) -> BoxFuture<'_, Result<PairingCreds, crate::AppError>> {
        Box::pin(pairing::refresh_bridge_jwt(c))
    }
    fn queue_revoke(&self, c: &PairingCreds) -> Result<(), crate::AppError> {
        pairing::queue_revoke(c)
    }
    fn retry_revokes(&self) -> BoxFuture<'_, Result<(), crate::AppError>> {
        Box::pin(pairing::retry_pending_revokes())
    }
}
impl StateHost for DesktopHost {
    fn catalog_epoch(&self) -> String {
        catalog::epoch()
    }
    fn sessions(&self, pair: &str) -> Option<(Value, String)> {
        catalog::sessions(pair)
    }
    fn workspaces(&self) -> Option<(Value, String)> {
        catalog::workspaces()
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
}
impl FileHost for DesktopHost {
    fn upload_chunk(&self, id: &str, index: u64, bytes: &[u8]) -> Result<Value, String> {
        files::write_upload_chunk(id, index, bytes)
    }
    fn download_chunk(&self, id: &str, index: u64) -> Result<Vec<u8>, crate::AppError> {
        files::read_download_chunk(id, index)
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

//! In-memory host for package-owned supervisor checks.
use super::*;
use crate::services::*;
use serde_json::Value;

type InvitePause = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<Invitation>,
);
#[derive(Default)]
pub(super) struct MemoryHost {
    pub slots: Mutex<Vec<PairingCreds>>,
    pub invitation: Mutex<Option<InvitePause>>,
    pub active: Mutex<Option<String>>,
    pub catalog_reads: Mutex<Vec<String>>,
    pub catalog_pause: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}
impl BusinessHost for MemoryHost {
    fn execute<'a>(
        &'a self,
        _: future_remote_protocol::IncomingCmd,
        reply: &'a dyn ReplySink,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            reply
                .send(false, Value::Null, Some("unused fixture business".into()))
                .await;
        })
    }
}
impl PairingHost for MemoryHost {
    fn load(&self) -> Option<PairingCreds> {
        self.slots.lock().unwrap().first().cloned()
    }
    fn load_slots(&self) -> Vec<PairingCreds> {
        self.slots.lock().unwrap().clone()
    }
    fn active_slot(&self) -> Option<String> {
        self.active.lock().unwrap().clone()
    }
    fn set_active(&self, pair: &str) -> Result<(), crate::Error> {
        *self.active.lock().unwrap() = Some(pair.into());
        Ok(())
    }
    fn remove_slot(&self, pair: &str) -> Result<(), crate::Error> {
        self.slots.lock().unwrap().retain(|c| c.pair_id != pair);
        Ok(())
    }
    fn save(&self, creds: &PairingCreds) -> Result<(), crate::Error> {
        let mut slots = self.slots.lock().unwrap();
        slots.retain(|c| c.pair_id != creds.pair_id);
        slots.push(creds.clone());
        Ok(())
    }
    fn clear(&self) -> Result<(), crate::Error> {
        self.slots.lock().unwrap().clear();
        Ok(())
    }
    fn invite(&self, _: bool) -> BoxFuture<'_, Result<Invitation, crate::Error>> {
        let pause = self.invitation.lock().unwrap().take();
        Box::pin(async move {
            let Some((entered, result)) = pause else {
                return Err(crate::Error::Message("unused fixture invitation".into()));
            };
            let _ = entered.send(());
            result
                .await
                .map_err(|_| crate::Error::Message("fixture invitation cancelled".into()))
        })
    }
    fn refresh(&self, creds: PairingCreds) -> BoxFuture<'_, Result<PairingCreds, crate::Error>> {
        Box::pin(async { Ok(creds) })
    }
    fn queue_revoke(&self, _: &PairingCreds) -> Result<(), crate::Error> {
        Ok(())
    }
    fn retry_revokes(&self) -> BoxFuture<'_, Result<(), crate::Error>> {
        Box::pin(async { Ok(()) })
    }
}
impl StateHost for MemoryHost {
    fn catalog_epoch(&self) -> String {
        "fixture_epoch".into()
    }
    fn sessions(&self, _: &str) -> Option<(Value, String)> {
        None
    }
    fn sessions_catalog(&self, pair: &str) -> Option<SessionsCatalog> {
        self.catalog_reads.lock().unwrap().push(pair.into());
        if let Some((entered, release)) = self.catalog_pause.lock().unwrap().take() {
            let _ = entered.send(());
            let _ = release.recv();
        }
        None
    }
    fn workspaces(&self) -> Option<(Value, String)> {
        None
    }
    fn catalog_revisions(&self) -> (String, u64, u64) {
        ("fixture_epoch".into(), 0, 0)
    }
    fn catalog_dirty(&self) -> bool {
        false
    }
    fn agent_available(&self) -> bool {
        false
    }
    fn monitor_agent(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
impl FileHost for MemoryHost {
    fn upload_chunk(&self, _: &str, _: u64, _: &[u8]) -> Result<Value, String> {
        Err("unused fixture file".into())
    }
    fn download_chunk(&self, _: &str, _: u64) -> Result<Vec<u8>, crate::Error> {
        Err(crate::Error::Message("unused fixture file".into()))
    }
    fn prune_transfers(&self) {}
    fn clear_transfers(&self) {}
    fn clear_preview_cache(&self) {}
}
pub(super) struct TokioExecutor;
impl TaskExecutor for TokioExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        tokio::spawn(future);
    }
    fn spawn_native(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(job);
    }
}
pub(super) fn isolated() -> Arc<Supervisor> {
    Supervisor::new(Arc::new(MemoryHost::default()), Arc::new(TokioExecutor))
}

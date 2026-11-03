//! Process-local protection of live durable stages. Durable receipts select
//! crash recovery; guards only prevent another live Manager reclaiming them.
use std::{collections::HashMap, sync::LazyLock};

static ACTIVE: LazyLock<parking_lot::Mutex<HashMap<String, usize>>> =
    LazyLock::new(Default::default);

pub(crate) struct ActiveOperation(String);
impl ActiveOperation {
    pub(crate) fn claim(operation: &str) -> Self {
        *ACTIVE.lock().entry(operation.to_owned()).or_default() += 1;
        Self(operation.to_owned())
    }
    pub(crate) fn is_active(operation: &str) -> bool {
        ACTIVE.lock().contains_key(operation)
    }
}
impl Drop for ActiveOperation {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock();
        if let Some(count) = active.get_mut(&self.0) {
            *count -= 1;
            if *count == 0 {
                active.remove(&self.0);
            }
        }
    }
}

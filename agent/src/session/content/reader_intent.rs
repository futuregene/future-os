//! Validate fields that would otherwise disappear from a duplicate's staging
//! manifest (for example large JSON -> {}, or nonempty text -> empty text).
//! Thin placeholder equality alone cannot establish immutable-body identity.
use super::*;

impl Snapshot {
    pub(crate) fn verify_existing_intent(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        input: &Value,
    ) -> Result<()> {
        let mut after = String::new();
        loop {
            let page = self.references(store, entry, &after)?;
            if page.is_empty() {
                break;
            }
            for reference in page {
                after = reference.field.clone();
                let expected = input
                    .pointer(&reference.field)
                    .context("conflicting entry identity: referenced field was removed")?;
                match reference.encoding {
                    Encoding::Text | Encoding::ShellSummary | Encoding::ToolArgumentsStringV1 => {
                        let text = expected
                            .as_str()
                            .context("conflicting entry identity: referenced text changed type")?;
                        ensure!(
                            !text.is_empty() || reference.range.length == 0,
                            "conflicting entry identity: referenced text was cleared"
                        );
                    }
                    Encoding::Json => {
                        // Larger values still enter stage_named and are checked
                        // against the immutable semantic/raw digest there.
                        // Small values might become inline, so validate them
                        // before any staged write can reuse the old placeholder.
                        if !is_inline(expected)? {
                            continue;
                        }
                        let (scope, locator, field) =
                            (self.clone(), entry.clone(), reference.field.clone());
                        let identity=store.db.read(move |db| {
                            let tx=db.transaction()?;scope.check(&tx)?;
                            let value=tx.query_row("SELECT o.json_sha256,o.sealed,r.byte_offset,r.byte_length,o.byte_length FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id JOIN content_objects o ON o.id=r.content_id WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND r.owner_kind='entry' AND r.field=?4",
                                params![scope.session,locator.position,locator.id,field],|row|Ok((row.get::<_,Option<String>>(0)?,row.get::<_,bool>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?)))?;
                            tx.commit()?;Ok(value)
                        })?;
                        if let (Some(hash), true, 0, length, total) = identity {
                            if length == total {
                                ensure!(
                                    super::super::json_digest(expected)? == hash,
                                    "conflicting entry JSON identity"
                                );
                                continue;
                            }
                        }
                        let actual =
                            decode(reference.encoding, self.body(store, entry, &reference)?)?;
                        ensure!(actual == *expected, "conflicting entry JSON identity");
                    }
                }
            }
        }
        self.verify(store)
    }
}

fn is_inline(value: &Value) -> Result<bool> {
    struct Counter {
        bytes: usize,
        over: bool,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.bytes.saturating_add(bytes.len())
                > super::super::extensions::INLINE_EXTENSION_BYTES
            {
                self.over = true;
                return Err(std::io::Error::other("inline JSON budget reached"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter {
        bytes: 0,
        over: false,
    };
    let result = serde_json::to_writer(&mut counter, value);
    if counter.over {
        return Ok(false);
    }
    result?;
    Ok(true)
}

//! Shared renewal ownership scenarios with explicit in-memory host services.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Suite {
    version: u32,
    suite: String,
    now_sec: i64,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Case {
    Expiry {
        id: String,
        expiry: i64,
        malformed: bool,
        expected: Languages<String>,
        difference: Option<String>,
    },
    Renewal {
        id: String,
        instances: usize,
        actions: Vec<Action>,
        expected: Expected,
        difference: Option<String>,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Languages<T> {
    rust: T,
    typescript: T,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    writes: Vec<usize>,
    fresh: Vec<bool>,
    continuations: Vec<usize>,
    closed: Vec<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    #[serde(rename = "type")]
    kind: ActionKind,
    target: usize,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ActionKind {
    Close,
    Retire,
    Supersede,
    Release,
}
impl Case {
    fn id(&self) -> &str {
        match self {
            Self::Expiry { id, .. } | Self::Renewal { id, .. } => id,
        }
    }
}
fn suite() -> Suite {
    let suite: Suite = serde_json::from_str(include_str!("../fixtures/credentials.json")).unwrap();
    assert_eq!(suite.version, 1, "unsupported credentials fixture version");
    assert_eq!(
        suite.suite, "client.credentials",
        "unsupported credentials fixture suite"
    );
    assert!(suite.now_sec > 0 && suite.now_sec <= 9_007_199_254_740);
    let mut ids = HashSet::new();
    for case in &suite.cases {
        assert!(
            case.id().starts_with("client.credentials.") && ids.insert(case.id()),
            "invalid/duplicate credential case ID"
        );
        match case {
            Case::Expiry {
                expiry,
                expected,
                difference,
                ..
            } => {
                assert!(expiry.unsigned_abs() <= 9_007_199_254_740_991);
                for decision in [&expected.rust, &expected.typescript] {
                    assert!(["refresh", "no_refresh", "invalid"].contains(&decision.as_str()));
                }
                if expected.rust != expected.typescript {
                    assert!(
                        difference
                            .as_ref()
                            .is_some_and(|note| !note.trim().is_empty()),
                        "undocumented policy difference"
                    );
                }
            }
            Case::Renewal {
                instances,
                actions,
                expected,
                difference,
                ..
            } => {
                assert!((1..=4).contains(instances));
                assert_eq!(expected.writes.len(), *instances);
                assert_eq!(expected.fresh.len(), *instances);
                assert_eq!(expected.continuations.len(), *instances);
                assert_eq!(expected.closed.len(), *instances);
                assert!(difference
                    .as_ref()
                    .is_none_or(|note| !note.trim().is_empty()));
                let mut releases = HashSet::new();
                for action in actions {
                    assert!(action.target < *instances);
                    if matches!(action.kind, ActionKind::Release) {
                        assert!(releases.insert(action.target));
                    }
                }
                assert_eq!(
                    releases.len(),
                    *instances,
                    "every host result must be resolved"
                );
            }
        }
    }
    suite
}
async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn shared_credential_policy_and_renewal_ownership() {
    let suite = suite();
    for case in &suite.cases {
        match case {
            Case::Expiry {
                id,
                expiry,
                malformed,
                expected,
                ..
            } => {
                let mut grant = creds();
                grant.jwt_expires_at = *expiry;
                grant.user_jwt = if *malformed {
                    "malformed_fixture".into()
                } else {
                    format!(
                        "header.{}.signature",
                        URL_SAFE_NO_PAD.encode(json!({"exp": expiry}).to_string())
                    )
                };
                let decision = if grant.needs_refresh(suite.now_sec) {
                    "refresh"
                } else {
                    "no_refresh"
                };
                assert_eq!(decision, expected.rust, "{id}");
            }
            Case::Renewal {
                id,
                instances,
                actions,
                expected,
                ..
            } => {
                let mut hosts = Vec::new();
                let mut clients = Vec::new();
                let mut pending = Vec::new();
                for _ in 0..*instances {
                    let host = Arc::new(MemoryHost::default());
                    let grant = creds();
                    host.upsert(grant.clone()).unwrap();
                    let client = Client::new(host.clone());
                    let epoch = client.begin_attempt(&grant.desktop_id).await.unwrap();
                    let caller = client.clone();
                    pending.push(tokio::spawn(async move {
                        caller.renew_credentials(&grant, epoch).await
                    }));
                    hosts.push(host);
                    clients.push(client);
                }
                for host in &hosts {
                    tokio::time::timeout(Duration::from_secs(1), host.entered.notified())
                        .await
                        .expect("host refresh did not enter");
                    assert_eq!(host.refreshes.load(Ordering::SeqCst), 1);
                }
                let mut released = HashSet::new();
                for action in actions {
                    let client = &clients[action.target];
                    let host = &hosts[action.target];
                    tokio::time::timeout(Duration::from_secs(1), async {
                        match action.kind {
                            ActionKind::Close => {
                                client.shutdown(Duration::from_secs(1)).await.unwrap();
                            }
                            ActionKind::Retire => client.disconnect("desktop_fixture").await,
                            ActionKind::Supersede => {
                                client.begin_attempt("desktop_fixture").await.unwrap();
                            }
                            ActionKind::Release => {
                                released.insert(action.target);
                                host.release.notify_one();
                            }
                        }
                    })
                    .await
                    .expect("renewal action blocked");
                    settle().await;
                    if matches!(action.kind, ActionKind::Close)
                        && !released.contains(&action.target)
                    {
                        assert!(
                            pending[action.target].is_finished(),
                            "{id}: stopped caller remained live"
                        );
                        assert_eq!(
                            host.finished_refreshes.load(Ordering::SeqCst),
                            1,
                            "{id}: cancelled host future was not dropped"
                        );
                    }
                }
                for (index, work) in pending.into_iter().enumerate() {
                    let result = tokio::time::timeout(Duration::from_secs(1), work)
                        .await
                        .expect("renewal caller did not finish")
                        .unwrap();
                    assert_eq!(
                        usize::from(result.is_ok()),
                        expected.continuations[index],
                        "{id}: continuation"
                    );
                    if let Ok(fresh) = result {
                        assert!(fresh.user_jwt == "new-token-fixture");
                    }
                    assert_eq!(
                        hosts[index].updates.load(Ordering::SeqCst),
                        expected.writes[index],
                        "{id}: writes"
                    );
                    assert_eq!(
                        hosts[index].load().unwrap().peers[0].creds.user_jwt == "new-token-fixture",
                        expected.fresh[index],
                        "{id}: persisted grant"
                    );
                    assert_eq!(
                        clients[index].is_stopped(),
                        expected.closed[index],
                        "{id}: wrong instance closed"
                    );
                    assert_eq!(hosts[index].finished_refreshes.load(Ordering::SeqCst), 1);
                    clients[index]
                        .shutdown(Duration::from_secs(1))
                        .await
                        .unwrap();
                }
                tokio::time::advance(Duration::from_secs(60)).await;
                for (index, host) in hosts.iter().enumerate() {
                    assert_eq!(
                        host.updates.load(Ordering::SeqCst),
                        expected.writes[index],
                        "{id}: late write"
                    );
                }
            }
        }
        println!("remote-client shared rust {} PASS", case.id());
    }
}

//! Shared skill management and recommendation operations.

use crate::{agent_bridge, skills};

/// Manually tell the agent to drop its skills cache and re-discover.
/// Use when entering the Skills page or on app startup so the
/// displayed list always reflects the current filesystem state.
pub async fn refresh_skills() -> Result<(), crate::Error> {
    agent_bridge::refresh_skills().await;
    Ok(())
}

pub async fn list_installed_skills() -> Result<Vec<agent_bridge::InstalledSkill>, crate::Error> {
    agent_bridge::list_installed_skills().await
}

pub async fn list_available_skills() -> Result<Vec<agent_bridge::AvailableSkill>, crate::Error> {
    agent_bridge::list_available_skills().await
}

/// Recommend at most one UNINSTALLED skill for the user's message via the
/// agent's Jev recommender. The caller decides when to invoke (toggle, length
/// bounds, login/balance, daily budget) and supplies the candidate set
/// (catalog − installed); this only forwards to the agent. Returns `None` on
/// refusal, timeout, error, or when the feature is unavailable server-side.
pub async fn suggest_skill(
    query: String,
    candidates: Vec<agent_bridge::SkillCandidate>,
) -> Result<Option<agent_bridge::SkillCandidate>, crate::Error> {
    agent_bridge::suggest_skill(&query, candidates).await
}

/// Today's recommendation state (count + already-shown skills and messages),
/// for the client's daily budget and duplicate checks.
pub async fn skill_reco_today() -> Result<crate::store::SkillRecoToday, crate::Error> {
    crate::store::skill_reco_today()
}

/// Record one recommendation that was actually shown to the user. Only shown
/// recommendations consume the daily budget; calls that recommend nothing must
/// not call this.
pub async fn record_skill_reco(skill_id: String, message_hash: String) -> Result<(), crate::Error> {
    crate::store::record_skill_reco(&skill_id, &message_hash)
}

/// The platform skill-guide config (coach prompt + manual link) for the
/// skill-onboarding banner. Unauthenticated platform call.
pub async fn get_skill_guide() -> Result<skills::SkillGuide, crate::Error> {
    skills::get_skill_guide().await
}

pub async fn install_skill(id: String, version: String) -> Result<(), crate::Error> {
    agent_bridge::install_skill(id, version).await?;
    crate::agent_events::publish_invalidation("skills_changed");
    Ok(())
}

pub async fn uninstall_skill(id: String) -> Result<bool, crate::Error> {
    let removed = agent_bridge::uninstall_skill(id).await?;
    if removed {
        crate::agent_events::publish_invalidation("skills_changed");
    }
    Ok(removed)
}

pub async fn sync_skills() -> Result<serde_json::Value, crate::Error> {
    let result = agent_bridge::sync_skills(true).await?;
    crate::agent_events::publish_invalidation("skills_changed");
    Ok(result)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::test_support::command_agent::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    /// `refresh_skills` is fire-and-forget (it reports success, not the agent's
    /// answer), and the daily-budget pair round-trips through the app store:
    /// only *shown* recommendations are recorded, and the count is the budget.
    #[tokio::test]
    async fn refresh_and_the_daily_recommendation_budget_round_trip() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-reco");
        crate::store::initialize_app_store().expect("init store");
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("refresh_skills".into(), "{}".into())]),
            ..Default::default()
        });
        refresh_skills().await.expect("refresh");

        let today = skill_reco_today().await.expect("today");
        assert_eq!(today.count, 0, "a fresh day has spent nothing");
        assert!(today.skill_ids.is_empty());
        assert!(today.message_hashes.is_empty());

        record_skill_reco("acme".into(), "hash-1".into())
            .await
            .expect("record");
        let today = skill_reco_today().await.expect("today");
        assert_eq!(today.count, 1);
        assert_eq!(today.skill_ids, vec!["acme".to_string()]);
        assert_eq!(today.message_hashes, vec!["hash-1".to_string()]);
        script_mock_agent(MockScript::default());
    }

    /// The recommender forwards the query and candidates to the agent and maps
    /// both answers a client can get: a suggestion, and a refusal (no skill).
    #[tokio::test]
    async fn a_skill_suggestion_is_forwarded_and_a_refusal_maps_to_none() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-suggest");
        crate::store::initialize_app_store().expect("init store");
        crate::test_support::command_agent::ensure_mock_agent();

        script_mock_agent(MockScript {
            data: HashMap::from([("suggest_skill".into(), "{}".into())]),
            ..Default::default()
        });
        let refused = suggest_skill("how do I x".into(), Vec::new())
            .await
            .expect("suggestion call");
        assert!(refused.is_none(), "a refusal is not an error");

        script_mock_agent(MockScript {
            data: HashMap::from([(
                "suggest_skill".into(),
                "{\"skill\":{\"name\":\"acme\",\"description\":\"does x\"}}".into(),
            )]),
            ..Default::default()
        });
        let suggested = suggest_skill(
            "how do I x".into(),
            vec![crate::agent_bridge::SkillCandidate {
                name: "acme".into(),
                description: "does x".into(),
            }],
        )
        .await
        .expect("suggestion call")
        .expect("a suggestion");
        assert_eq!(suggested.name, "acme");
        script_mock_agent(MockScript::default());
    }

    /// The skill page's own reads and the three management calls, as the window
    /// reaches them. The invalidation each write publishes is the reason these
    /// wrappers exist at all: the page reloads because it hears the event, not
    /// because the call returned.
    #[tokio::test]
    async fn the_skill_page_reads_and_writes_reach_the_agent() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-page");
        crate::store::initialize_app_store().expect("init store");
        crate::test_support::command_agent::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([
                (
                    "list_installed_skills".into(),
                    r#"[{"id":"my-skill","name":"my-skill","description":"does things","version":"1.2.3"}]"#
                        .into(),
                ),
                (
                    "list_available_skills".into(),
                    r#"[{"id":"future-x","name":"X","latestVersion":"2.0.0","builtin":true,"upgradeAvailable":true}]"#
                        .into(),
                ),
                ("install_skill".into(), "{}".into()),
                ("uninstall_skill".into(), r#"{"removed":true}"#.into()),
                (
                    "sync_skills".into(),
                    r#"{"installed":[],"upgraded":[],"skipped":[],"failed":[]}"#.into(),
                ),
            ]),
            ..Default::default()
        });

        let installed = list_installed_skills().await.expect("installed");
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].version.as_deref(), Some("1.2.3"));
        let available = list_available_skills().await.expect("available");
        assert_eq!(available.len(), 1);
        assert!(available[0].upgrade_available);

        install_skill("future-x".into(), "2.0.0".into())
            .await
            .expect("install");
        assert!(
            uninstall_skill("future-x".into()).await.expect("uninstall"),
            "the agent removed it"
        );
        let synced = sync_skills().await.expect("sync");
        assert_eq!(synced["installed"], serde_json::json!([]));

        // An uninstall that removed nothing must still answer `false` rather than
        // reporting a change the page would have to undo.
        script_mock_agent(MockScript {
            data: HashMap::from([("uninstall_skill".into(), r#"{"removed":false}"#.into())]),
            ..Default::default()
        });
        assert!(!uninstall_skill("absent".into()).await.expect("uninstall"));
        script_mock_agent(MockScript::default());
    }
}

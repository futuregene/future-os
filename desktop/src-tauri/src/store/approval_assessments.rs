//! Immutable projection of automatic reviews. Never creates a pending request.
use super::db::*;
use super::util::*;
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAssessmentRecord {
    pub id: String,
    pub run_id: String,
    pub tool_call_id: String,
    pub status: String,
    pub payload: Value,
    pub created_at: i64,
}

pub fn record_approval_assessment(run_id: &str, value: &Value) -> Result<(), crate::AppError> {
    let field = |key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("Missing assessment {key}"))
    };
    let id = field("assessment_id")?;
    let request_id = field("approval_request_id")?;
    let tool_id = field("tool_call_id")?;
    let status = field("status")?;
    if !matches!(
        status,
        "approved"
            | "rejected"
            | "review_error"
            | "review_uncertain"
            | "cancelled"
            | "stale_request"
    ) || field("reviewer")? != "model"
    {
        return Err("Invalid automatic assessment".to_string().into());
    }
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let thread_id = run_thread_id(&tx, run_id)?;
    let now = now_millis();
    // Review failures are terminal denials in the approval queue; the richer
    // status remains in the assessment itself. Replay cannot rewrite either.
    let request_status = match status {
        "approved" => "approved",
        "cancelled" | "stale_request" => "cancelled",
        _ => "rejected",
    };
    let action = value.get("action").cloned().unwrap_or(Value::Null);
    tx.execute("INSERT OR IGNORE INTO approval_requests (id,thread_id,run_id,tool_call_id,kind,status,title,risk_level,action_payload,reviewer,decision_scope,decision_source,decided_at,created_at,updated_at) VALUES (?1,?2,?3,?4,'automatic',?5,'Automatic review',?6,?7,'model','once','auto_review',?8,?8,?8)", params![request_id,thread_id,run_id,tool_id,request_status,value.pointer("/effective/risk").and_then(Value::as_str),action.to_string(),now])?;
    let owner: (String, String) = tx.query_row(
        "SELECT run_id, reviewer FROM approval_requests WHERE id=?1",
        [request_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if owner != (run_id.to_owned(), "model".to_owned()) {
        return Err("Assessment request ownership mismatch".to_string().into());
    }
    tx.execute("INSERT OR IGNORE INTO approval_assessments (id,approval_request_id,run_id,tool_call_id,status,payload,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![id,request_id,run_id,tool_id,status,value.to_string(),now])?;
    tx.commit()?;
    Ok(())
}

pub fn list_approval_assessments(
    run_id: &str,
) -> Result<Vec<ApprovalAssessmentRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare("SELECT id,run_id,tool_call_id,status,payload,created_at FROM approval_assessments WHERE run_id=?1 ORDER BY created_at,id")?;
    let rows = stmt.query_map([run_id], |r| {
        let payload: String = r.get(4)?;
        Ok(ApprovalAssessmentRecord {
            id: r.get(0)?,
            run_id: r.get(1)?,
            tool_call_id: r.get(2)?,
            status: r.get(3)?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::super::db::test_support::guarded_conn;
    use super::*;
    use serde_json::json;
    #[test]
    fn assessment_is_terminal_immutable_and_does_not_pause_run() {
        let (_home, conn) = guarded_conn("auto_assessments");
        conn.execute_batch("INSERT INTO workspaces(id,name,kind,path,created_at,updated_at) VALUES('w','W','temporary','fixture',1,1); INSERT INTO threads(id,workspace_id,mode,title,created_at,updated_at) VALUES('t','w','chat','T',1,1); INSERT INTO runs(id,thread_id,status,created_at,updated_at) VALUES('r','t','running',1,1);").unwrap();
        let mut value = json!({"assessment_id":"a","approval_request_id":"a","tool_call_id":"tool","reviewer":"model","status":"approved","effective":{"risk":"low"},"action":{"command":"pwd"}});
        record_approval_assessment("r", &value).unwrap();
        value["status"] = json!("rejected");
        record_approval_assessment("r", &value).unwrap();
        let entries = list_approval_assessments("r").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "approved");
        assert_eq!(
            super::super::get_approval_request("a")
                .unwrap()
                .unwrap()
                .status,
            "approved"
        );
        assert!(super::super::list_pending_approval_requests()
            .unwrap()
            .is_empty());
        assert_eq!(
            super::super::get_run("r").unwrap().unwrap().status,
            "running"
        );
        conn.execute("DELETE FROM approval_requests WHERE id='a'", [])
            .unwrap();
        assert!(list_approval_assessments("r").unwrap().is_empty());
        value["assessment_id"] = json!("b");
        value["approval_request_id"] = json!("b");
        record_approval_assessment("missing-run", &value).unwrap_err();
        assert!(list_approval_assessments("missing-run").unwrap().is_empty());
    }
}

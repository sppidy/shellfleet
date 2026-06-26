use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::AppState;
use sqlx::SqlitePool;

/// Same semantics as ee/src/acl.rs:matches_action:
/// - `*` matches any action
/// - `prefix:*` matches any action starting with `prefix:`
/// - Everything else requires an exact match.
fn matches_action(pattern: &str, action: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.ends_with(":*") {
        let prefix = &pattern[..pattern.len() - 1];
        return action.starts_with(prefix);
    }
    pattern == action
}

#[derive(Debug, sqlx::FromRow)]
struct IamStatement {
    effect: String,
    actions: String,   // JSON array string
    resources: String, // JSON array string
}

pub async fn middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    // Strip any client-supplied x-api-key-* headers so they can only be
    // injected below by a valid sf_live_ key lookup. Non-API-key requests
    // reach handlers with no x-api-key-* headers present.
    let strip_headers = req.headers_mut();
    strip_headers.remove("x-api-key-login");
    strip_headers.remove("x-api-key-role");
    strip_headers.remove("x-api-key-policy-id");

    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let Some(ref token) = auth_header else {
        return next.run(req).await;
    };

    if !token.starts_with("sf_live_") {
        return next.run(req).await;
    }

    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    };
    let now = crate::now_unix();

    #[derive(sqlx::FromRow)]
    struct KeyRow {
        id: i64,
        login: String,
        policy_id: Option<i64>,
        expires_at: Option<i64>,
    }

    let row: Option<KeyRow> = sqlx::query_as(
        "SELECT id, login, policy_id, expires_at FROM ee_api_keys WHERE key_hash = ?1 AND revoked = 0",
    )
    .bind(&hash)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let Some(row) = row else {
        return (StatusCode::UNAUTHORIZED, "invalid API key").into_response();
    };

    if let Some(exp) = row.expires_at {
        if exp < now {
            return (StatusCode::UNAUTHORIZED, "API key expired").into_response();
        }
    }

    let _ = sqlx::query("UPDATE ee_api_keys SET last_used_at = ?1 WHERE id = ?2")
        .bind(now)
        .bind(row.id)
        .execute(&state.db)
        .await;

    let role = match crate::db::get_user(&state.db, &row.login).await {
        Ok(Some(user)) => user.role,
        _ => "viewer".to_string(),
    };

    let headers = req.headers_mut();
    if let Ok(v) = axum::http::HeaderValue::from_str(&row.login) {
        headers.insert(axum::http::HeaderName::from_static("x-api-key-login"), v);
    }
    if let Ok(v) = axum::http::HeaderValue::from_str(&role) {
        headers.insert(axum::http::HeaderName::from_static("x-api-key-role"), v);
    }
    if let Some(pid) = row.policy_id {
        if let Ok(v) = axum::http::HeaderValue::from_str(&pid.to_string()) {
            headers.insert(
                axum::http::HeaderName::from_static("x-api-key-policy-id"),
                v,
            );
        }
    }

    next.run(req).await
}

/// Check that the API key's bound IAM policy allows `required_action`.
/// Null policy_id → skip (role-based fallback, unchanged).
pub async fn require_key_action(
    headers: &HeaderMap,
    pool: &SqlitePool,
    required_action: &str,
) -> Result<(), (StatusCode, String)> {
    let policy_id = headers
        .get("x-api-key-policy-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok());
    let Some(pid) = policy_id else {
        return Ok(());
    };

    let rows = sqlx::query_as::<_, IamStatement>(
        "SELECT effect, actions, resources FROM ee_iam_statements WHERE policy_id = ?1",
    )
    .bind(pid)
    .fetch_all(pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to load policy statements".into(),
        )
    })?;

    // Fail closed: exactly `["*"]` resources accepted. Malformed, empty,
    // or mixed resources → 500.
    for row in &rows {
        let resources: Vec<String> = match serde_json::from_str(&row.resources) {
            Ok(r) => r,
            Err(_) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "malformed policy resources".into(),
                ));
            }
        };
        if resources.len() != 1 || resources[0] != "*" {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "resource-constrained policies cannot bind to API keys".into(),
            ));
        }
    }

    // Deny takes precedence, then allow. Default-deny. Malformed actions
    // fail closed so a broken Deny row is never silently ignored.
    let mut allowed = false;
    for row in &rows {
        let actions: Vec<String> = match serde_json::from_str(&row.actions) {
            Ok(a) => a,
            Err(_) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "malformed policy actions".into(),
                ));
            }
        };
        let matches = actions.iter().any(|a| matches_action(a, required_action));
        if matches {
            match row.effect.as_str() {
                "Deny" => {
                    return Err((
                        StatusCode::FORBIDDEN,
                        format!("API key scope '{required_action}' is denied by its bound policy"),
                    ));
                }
                "Allow" => allowed = true,
                _ => {}
            }
        }
    }
    if allowed {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!("API key scope '{required_action}' is not permitted by its bound policy"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_action_wildcard_any() {
        assert!(matches_action("*", "agent:View"));
        assert!(matches_action("*", "k8s:Delete"));
    }

    #[test]
    fn matches_action_prefix_star() {
        assert!(matches_action("agent:*", "agent:View"));
        assert!(matches_action("agent:*", "agent:Terminal"));
        assert!(matches_action("backup:*", "backup:Run"));
        assert!(!matches_action("agent:*", "k8s:List"));
    }

    #[test]
    fn matches_action_exact() {
        assert!(matches_action("k8s:List", "k8s:List"));
        assert!(!matches_action("k8s:List", "k8s:Describe"));
    }

    async fn setup_iam_db() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS ee_iam_statements (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                policy_id INTEGER NOT NULL,
                effect TEXT NOT NULL CHECK (effect IN ('Allow', 'Deny')),
                actions TEXT NOT NULL DEFAULT '[]',
                resources TEXT NOT NULL DEFAULT '[\"*\"]'
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn deny_takes_precedence_over_allow() {
        let pool = setup_iam_db().await;
        // Policy 1: Allow agent:View, Deny agent:View → deny wins
        sqlx::query(
            "INSERT INTO ee_iam_statements (policy_id, effect, actions, resources) VALUES (1, 'Allow', '[\"agent:View\"]', '[\"*\"]')"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO ee_iam_statements (policy_id, effect, actions, resources) VALUES (1, 'Deny', '[\"agent:View\"]', '[\"*\"]')"
        ).execute(&pool).await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key-policy-id", "1".parse().unwrap());
        let res = require_key_action(&headers, &pool, "agent:View").await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn default_deny_when_action_not_listed() {
        let pool = setup_iam_db().await;
        // Policy 2: Allow agent:View only → agent:Terminal not listed → 403
        sqlx::query(
            "INSERT INTO ee_iam_statements (policy_id, effect, actions, resources) VALUES (2, 'Allow', '[\"agent:View\"]', '[\"*\"]')"
        ).execute(&pool).await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key-policy-id", "2".parse().unwrap());
        let res = require_key_action(&headers, &pool, "agent:Terminal").await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn wildcard_prefix_allows_any_action_in_prefix() {
        let pool = setup_iam_db().await;
        // Policy 3: Allow agent:* → all agent:* actions pass
        sqlx::query(
            "INSERT INTO ee_iam_statements (policy_id, effect, actions, resources) VALUES (3, 'Allow', '[\"agent:*\"]', '[\"*\"]')"
        ).execute(&pool).await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key-policy-id", "3".parse().unwrap());
        assert!(
            require_key_action(&headers, &pool, "agent:View")
                .await
                .is_ok()
        );
        assert!(
            require_key_action(&headers, &pool, "agent:Terminal")
                .await
                .is_ok()
        );
        assert!(
            require_key_action(&headers, &pool, "agent:Exec")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn malformed_actions_fail_closed() {
        let pool = setup_iam_db().await;
        sqlx::query(
            "INSERT INTO ee_iam_statements (policy_id, effect, actions, resources) VALUES (4, 'Allow', 'not-json', '[\"*\"]')"
        ).execute(&pool).await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key-policy-id", "4".parse().unwrap());
        let res = require_key_action(&headers, &pool, "agent:View").await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn malformed_resources_fail_closed() {
        let pool = setup_iam_db().await;
        sqlx::query(
            "INSERT INTO ee_iam_statements (policy_id, effect, actions, resources) VALUES (5, 'Allow', '[\"agent:View\"]', 'not-json')"
        ).execute(&pool).await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key-policy-id", "5".parse().unwrap());
        let res = require_key_action(&headers, &pool, "agent:View").await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn mixed_resources_fail_closed() {
        let pool = setup_iam_db().await;
        sqlx::query(
            "INSERT INTO ee_iam_statements (policy_id, effect, actions, resources) VALUES (6, 'Allow', '[\"agent:View\"]', '[\"*\",\"prod-*\"]')"
        ).execute(&pool).await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key-policy-id", "6".parse().unwrap());
        let res = require_key_action(&headers, &pool, "agent:View").await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().0, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

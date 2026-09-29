//! 管理面处理函数。

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

use crate::auth::{AuthError, RequireAdmin};
use crate::http::AppState;

/// 一页最多 200 条：管理接口是给运维看的，不该被用来全量导出（导出是另一件事）。
const MAX_PAGE_SIZE: i64 = 200;

#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListUsersResponse {
    pub users: Vec<crate::auth::User>,
    pub limit: i64,
    pub offset: i64,
}

/// `GET /api/v1/admin/users`：分页列出账号（新建在前）。
pub async fn list_users(
    State(state): State<AppState>,
    admin: RequireAdmin,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<ListUsersResponse>, AuthError> {
    let limit = query.limit.unwrap_or(50).clamp(1, MAX_PAGE_SIZE);
    let offset = query.offset.unwrap_or(0).max(0);

    let users = crate::auth::list_users(&state.pool, limit, offset).await?;
    tracing::debug!(admin_id = %admin.0.user.id, count = users.len(), "管理面查询账号列表");

    Ok(Json(ListUsersResponse {
        users,
        limit,
        offset,
    }))
}

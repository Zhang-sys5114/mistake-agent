//! S2 验收：账号、令牌与角色边界（ADR-0047 决策 3/4）。
//!
//! 覆盖验收标准点名的四类情形——**越权、令牌撤销、令牌过期、角色边界**——
//! 外加注册/登录的正常与异常路径。需要可写 PostgreSQL，连接方式见 `tests/common/mod.rs`。

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const PASSWORD: &str = "correct-horse-battery";

// ---------- 请求辅助 ----------

async fn call(
    app: Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = match body {
        Some(value) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(value.to_string()))
            .expect("请求构造失败"),
        None => builder.body(Body::empty()).expect("请求构造失败"),
    };
    let res = app.oneshot(request).await.expect("路由调用失败");
    let status = res.status();
    let bytes = res
        .into_body()
        .collect()
        .await
        .expect("读取响应体失败")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn register(app: &Router, email: &str) -> (StatusCode, Value) {
    call(
        app.clone(),
        "POST",
        "/api/v1/auth/register",
        None,
        Some(json!({"email": email, "password": PASSWORD, "display_name": "小明"})),
    )
    .await
}

async fn login(app: &Router, email: &str, password: &str) -> (StatusCode, Value) {
    call(
        app.clone(),
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"email": email, "password": password})),
    )
    .await
}

/// 注册并登录，返回（邮箱，令牌）。
async fn register_and_login(app: &Router) -> (String, String) {
    let email = common::unique_email();
    let (status, _) = register(app, &email).await;
    assert_eq!(status, StatusCode::CREATED, "注册应返回 201");

    let (status, body) = login(app, &email, PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "登录应返回 200：{body}");
    (
        email,
        body["token"].as_str().expect("响应缺少 token").to_string(),
    )
}

// ---------- 注册 ----------

#[tokio::test]
async fn register_creates_plain_user_with_sync_disabled() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let email = common::unique_email();
    let (status, body) = register(&app, &email).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["user"]["email"], email);
    assert_eq!(body["user"]["role"], "user", "自助注册只能是普通用户");
    assert_eq!(
        body["user"]["sync_enabled"], false,
        "同步默认关闭（ADR-0049）"
    );
    assert_eq!(body["user"]["disabled"], false);
    // 口令哈希绝不能出现在响应里
    assert!(!body.to_string().contains("$argon2"));
    assert!(body["user"]["id"].is_string());
}

#[tokio::test]
async fn register_rejects_duplicate_email_case_insensitively() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let email = common::unique_email();
    assert_eq!(register(&app, &email).await.0, StatusCode::CREATED);

    // 大小写不同视为同一账号（与 users_email_lower_key 同口径）
    let (status, body) = register(&app, &email.to_uppercase()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "email_taken");
}

#[tokio::test]
async fn register_validates_email_and_password() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let (status, body) = register(&app, "not-an-email").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_failed");

    let (status, _) = call(
        app.clone(),
        "POST",
        "/api/v1/auth/register",
        None,
        Some(json!({"email": common::unique_email(), "password": "short"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "短口令应被拒绝");
}

// ---------- 登录 ----------

#[tokio::test]
async fn login_returns_token_that_authenticates() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let email = common::unique_email();
    register(&app, &email).await;

    let (status, body) = login(&app, &email, PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("响应缺少 token");
    assert!(token.starts_with("mka_"), "令牌应带前缀：{token}");
    assert!(body["expires_at"].is_string(), "应返回过期时间");
    assert_eq!(body["user"]["email"], email);

    let (status, me) = call(app, "GET", "/api/v1/me", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["user"]["email"], email);
}

#[tokio::test]
async fn login_failures_are_indistinguishable() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let email = common::unique_email();
    register(&app, &email).await;

    // 口令错 与 账号不存在 必须给出**完全相同**的响应，否则登录接口就成了邮箱枚举器
    let (wrong_password_status, wrong_password) = login(&app, &email, "definitely-wrong").await;
    let (unknown_status, unknown) = login(&app, &common::unique_email(), PASSWORD).await;

    assert_eq!(wrong_password_status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_password, unknown);
    assert_eq!(wrong_password["error"]["code"], "invalid_credentials");
}

// ---------- 鉴权与令牌生命周期 ----------

#[tokio::test]
async fn me_requires_a_valid_token() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let (status, body) = call(app.clone(), "GET", "/api/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "missing_token");

    for bad in ["Bearer-not-our-format", "mka_zzzz", "mka_"] {
        let (status, body) = call(app.clone(), "GET", "/api/v1/me", Some(bad), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{bad} 应被拒绝");
        assert!(
            matches!(
                body["error"]["code"].as_str(),
                Some("invalid_token" | "missing_token")
            ),
            "{bad} → {body}"
        );
    }

    // 形状合法但从未签发的令牌：必须查库后拒绝（不能靠形状蒙过去）
    let fake = format!("mka_{}", "0".repeat(64));
    let (status, body) = call(app, "GET", "/api/v1/me", Some(&fake), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_token");
}

#[tokio::test]
async fn logout_revokes_only_the_presented_token() {
    let pool = common::test_pool().await;
    let app = common::app(pool.clone());

    let (email, first) = register_and_login(&app).await;
    let (_, second) = login(&app, &email, PASSWORD).await;
    let second = second["token"].as_str().unwrap().to_string();

    let (status, _) = call(
        app.clone(),
        "POST",
        "/api/v1/auth/logout",
        Some(&first),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 被登出的令牌立即失效
    let (status, body) = call(app.clone(), "GET", "/api/v1/me", Some(&first), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_token");

    // 另一台设备的令牌不受影响
    let (status, _) = call(app.clone(), "GET", "/api/v1/me", Some(&second), None).await;
    assert_eq!(status, StatusCode::OK, "登出不该影响其它设备的令牌");

    // 库里确实记了撤销时间（而不是仅靠内存态）
    let revoked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tokens WHERE revoked_at IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked >= 1);
}

#[tokio::test]
async fn expired_token_is_rejected() {
    let pool = common::test_pool().await;
    let app = common::app(pool.clone());

    let (email, token) = register_and_login(&app).await;

    // 直接把过期时间推到过去：等价于"等 90 天"，但不用真的等。
    // 必须**按用户限定**——测试并行跑在同一个库上，无 WHERE 的 UPDATE 会把
    // 其它测试刚签发的令牌一起过期掉（真踩过这个坑）。
    let affected = sqlx::query(
        "UPDATE tokens SET expires_at = now() - interval '1 second'
         WHERE user_id = (SELECT id FROM users WHERE lower(email) = lower($1))",
    )
    .bind(&email)
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected();
    assert!(affected >= 1, "应当更新到该用户刚签发的令牌");

    let (status, body) = call(app, "GET", "/api/v1/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "过期令牌必须被拒绝");
    assert_eq!(body["error"]["code"], "invalid_token");
}

#[tokio::test]
async fn disabled_account_is_rejected_with_its_own_code() {
    let pool = common::test_pool().await;
    let app = common::app(pool.clone());

    let (email, token) = register_and_login(&app).await;
    sqlx::query("UPDATE users SET status = 'disabled' WHERE lower(email) = lower($1)")
        .bind(&email)
        .execute(&pool)
        .await
        .unwrap();

    // 令牌本身有效，是账号被停用——错误码要能与"令牌失效"区分开
    let (status, body) = call(app.clone(), "GET", "/api/v1/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "account_disabled");

    // 停用账号也不能再登录
    let (status, _) = login(&app, &email, PASSWORD).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---------- 账号状态更新 ----------

#[tokio::test]
async fn update_me_toggles_sync_and_clears_display_name() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let (_, token) = register_and_login(&app).await;

    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/v1/me",
        Some(&token),
        Some(json!({"sync_enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["sync_enabled"], true);
    assert_eq!(
        body["user"]["display_name"], "小明",
        "未提供的字段不该被清空"
    );

    // 空串 = 清空展示名（与客户端 nickname 语义一致）
    let (status, body) = call(
        app.clone(),
        "PATCH",
        "/api/v1/me",
        Some(&token),
        Some(json!({"display_name": "   "})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["display_name"], "");
    assert_eq!(body["user"]["sync_enabled"], true, "局部更新不该动其它字段");

    // 超长展示名被拒
    let (status, _) = call(
        app,
        "PATCH",
        "/api/v1/me",
        Some(&token),
        Some(json!({"display_name": "字".repeat(25)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------- 角色边界与越权 ----------

#[tokio::test]
async fn regular_user_cannot_reach_admin_surface() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let (_, token) = register_and_login(&app).await;

    let (status, body) = call(app, "GET", "/api/v1/admin/users", Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "普通用户访问管理面必须被拒");
    assert_eq!(body["error"]["code"], "forbidden");
}

#[tokio::test]
async fn admin_surface_requires_authentication_at_all() {
    let pool = common::test_pool().await;
    let app = common::app(pool);

    let (status, body) = call(app, "GET", "/api/v1/admin/users", None, None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "未登录应先是 401 而不是 403"
    );
    assert_eq!(body["error"]["code"], "missing_token");
}

#[tokio::test]
async fn seeded_admin_can_list_accounts() {
    let pool = common::test_pool().await;
    let app = common::app(pool.clone());

    // 走真实的管理员种子路径（ADR-0047 决策 3），顺带验证它幂等
    let admin_email = common::unique_email();
    let admin_password = "admin-password-123";
    let cfg = common::config_with_admin(&admin_email, admin_password);
    mistake_agent_server::auth::bootstrap_admin(&pool, &cfg)
        .await
        .expect("创建管理员种子失败");
    mistake_agent_server::auth::bootstrap_admin(&pool, &cfg)
        .await
        .expect("重复创建管理员种子不应报错");

    let (status, body) = login(&app, &admin_email, admin_password).await;
    assert_eq!(status, StatusCode::OK, "管理员应能登录：{body}");
    assert_eq!(body["user"]["role"], "admin");
    let token = body["token"].as_str().unwrap();

    // 造一个普通用户，确认列表里能看到（并且看得到角色差异）
    let member_email = common::unique_email();
    register(&app, &member_email).await;

    let (status, body) = call(
        app,
        "GET",
        "/api/v1/admin/users?limit=200",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let users = body["users"].as_array().expect("响应缺少 users 数组");
    let emails: Vec<&str> = users.iter().filter_map(|u| u["email"].as_str()).collect();
    assert!(emails.contains(&admin_email.as_str()), "列表应含管理员");
    assert!(
        emails.contains(&member_email.as_str()),
        "列表应含刚注册的用户"
    );
    assert!(
        users
            .iter()
            .any(|u| u["email"] == admin_email && u["role"] == "admin"),
        "管理员角色必须如实返回"
    );
}

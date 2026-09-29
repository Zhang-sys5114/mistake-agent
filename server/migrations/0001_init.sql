-- 0001_init：账号与令牌基础表（ADR-0047 决策 3/4）
--
-- 角色与状态用 text + CHECK 而非 Postgres 枚举类型：加角色不需要 ALTER TYPE，演进成本更低。
-- 时间统一 timestamptz（服务端与客户端一样本地优先，但服务端跨时区部署必须带时区）。

CREATE TABLE users (
    id            uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    email         text        NOT NULL,
    password_hash text        NOT NULL,
    role          text        NOT NULL DEFAULT 'user'
                              CHECK (role IN ('user', 'teacher', 'admin')),
    display_name  text        NOT NULL DEFAULT '',
    status        text        NOT NULL DEFAULT 'active'
                              CHECK (status IN ('active', 'disabled')),
    -- 设备数据同步开关（ADR-0049）：默认关闭，登录后 OOBE 询问
    sync_enabled  boolean     NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

-- 邮箱大小写不敏感唯一：存原文，按 lower(email) 建唯一索引
CREATE UNIQUE INDEX users_email_lower_key ON users (lower(email));

CREATE TABLE tokens (
    id           uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 只存 SHA-256，明文令牌永不落库（ADR-0047 决策 4）
    token_hash   bytea       NOT NULL UNIQUE,
    label        text        NOT NULL DEFAULT '',
    created_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz,
    last_used_at timestamptz,
    revoked_at   timestamptz
);

CREATE INDEX tokens_user_id_idx ON tokens (user_id);

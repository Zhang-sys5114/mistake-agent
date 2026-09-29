-- 0002_billing：套餐、权益与用量流水（ADR-0047 决策 6/7，修订 R2/R4/R6）
--
-- 为什么这三张表落在 S3 而不是 S4（R6）：中转没有权益就无从限流，只能一律 402。
-- S4 只保留兑换码、admin CLI、CSV 导出与套餐数值校准。

-- 商品定义。窗口阈值可**热改**（改库即生效，不用发版），因此刻意不做缓存快照。
CREATE TABLE plans (
    id            uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    code          text        NOT NULL UNIQUE,
    name          text        NOT NULL,
    kind          text        NOT NULL CHECK (kind IN ('experience', 'monthly')),
    price_cents   integer     NOT NULL DEFAULT 0,
    -- 生效天数：体验包也给期限，这样"先扣最早到期的权益"这条规则对所有套餐都成立
    duration_days integer     NOT NULL CHECK (duration_days > 0),
    -- 体验包：一次性总次数；月卡：NULL（由窗口阈值约束）
    total_uses    integer     CHECK (total_uses IS NULL OR total_uses > 0),
    -- 三滑动窗口的次数上限（NULL = 该窗口不限制）
    limit_5h      integer     CHECK (limit_5h IS NULL OR limit_5h > 0),
    limit_week    integer     CHECK (limit_week IS NULL OR limit_week > 0),
    limit_month   integer     CHECK (limit_month IS NULL OR limit_month > 0),
    device_limit  integer     NOT NULL DEFAULT 1 CHECK (device_limit > 0),
    concurrency   integer     NOT NULL DEFAULT 2 CHECK (concurrency > 0),
    enabled       boolean     NOT NULL DEFAULT true,
    sort          integer     NOT NULL DEFAULT 0,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    -- 两类套餐的形状不同，用 CHECK 把"半成品"挡在库外
    CHECK (kind <> 'experience' OR total_uses IS NOT NULL),
    CHECK (kind <> 'monthly' OR total_uses IS NULL)
);

-- 权益：一次兑换/发放产生的可用额度实例。
CREATE TABLE entitlements (
    id          uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    plan_id     uuid        NOT NULL REFERENCES plans (id),
    source      text        NOT NULL CHECK (source IN ('redeem', 'grant', 'payment')),
    started_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL,
    -- 发放时**快照**套餐的总次数：之后改套餐不该回头改已发出的权益
    -- （窗口阈值相反，取自 plans 实时值，以便"改库即收紧"）
    total_uses  integer,
    used_uses   integer     NOT NULL DEFAULT 0 CHECK (used_uses >= 0),
    status      text        NOT NULL DEFAULT 'active'
                            CHECK (status IN ('active', 'expired', 'exhausted', 'revoked')),
    note        text        NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- 计费只关心"还没到期且还生效"的权益，走部分索引
CREATE INDEX entitlements_user_active_idx
    ON entitlements (user_id, expires_at) WHERE status = 'active';

-- 用量流水：既是三窗口的聚合源，也是成本核算的账本。
CREATE TABLE usage_events (
    id               bigserial   PRIMARY KEY,
    user_id          uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 权益删除后流水仍要留档，故 SET NULL
    entitlement_id   uuid        REFERENCES entitlements (id) ON DELETE SET NULL,
    -- 令牌不加外键：令牌被清理后流水依然要能追溯
    token_id         uuid,
    request_id       text        NOT NULL,
    -- 下游协议面（responses / chat_completions / anthropic）：用于分协议看成本
    protocol         text        NOT NULL,
    model            text        NOT NULL,
    -- token 明细只做内账（R4 的四元组），不对外露出
    input_tokens     bigint,
    cached_tokens    bigint,
    output_tokens    bigint,
    reasoning_tokens bigint,
    -- 对外的"次数"：reserved 阶段先记 1，结算时按阶梯上调或归零（R2）
    billed_uses      integer     NOT NULL DEFAULT 0 CHECK (billed_uses >= 0),
    latency_ms       integer,
    status           text        NOT NULL
                                 CHECK (status IN ('reserved', 'ok', 'upstream_error', 'aborted')),
    created_at       timestamptz NOT NULL DEFAULT now()
);

-- 三滑动窗口按 (user_id, created_at) 聚合，这是最热的查询
CREATE INDEX usage_events_user_created_idx ON usage_events (user_id, created_at DESC);
CREATE INDEX usage_events_entitlement_idx ON usage_events (entitlement_id);

-- 套餐种子：数值是**占位**，S3 上线后用真实用量按成本反推再收紧（改库即生效）。
-- 窗口阈值按"5 小时 / 7 天 / 30 天"三档给，体验包只受总次数约束。
INSERT INTO plans (code, name, kind, price_cents, duration_days, total_uses,
                   limit_5h, limit_week, limit_month, device_limit, concurrency, sort)
VALUES
    ('experience_10', '体验包（10 次）', 'experience',  100, 30, 10, NULL, NULL, NULL, 1, 2, 10),
    ('monthly_lite',  '月卡 Lite',      'monthly',     2800, 30, NULL,   15,  100,  300, 3, 2, 20),
    ('monthly_pro',   '月卡 Pro',       'monthly',     6800, 30, NULL,   40,  280,  900, 3, 2, 30),
    ('monthly_max',   '月卡 Max',       'monthly',    12800, 30, NULL,   90,  600, 2000, 5, 2, 40)
ON CONFLICT (code) DO NOTHING;

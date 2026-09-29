# 0049 — 多设备同步协议（outbox + change log）

日期：2026-09-29
状态：已采纳
依赖：ADR-0047（服务端架构）、ADR-0048（客户端接入平台服务）
修订：ADR-0039/0042（错题以目录为领域对象的形态在服务端映射为结构化行）、ADR-0040（`schedule.json` 是事件流的折叠，故**不参与同步**）

## 背景

账号体系（ADR-0047/0048）落地后，同一账号会在多台设备/多端上使用（家里的 PC、学校电脑、将来的移动端）。用户要求首期即实现设备数据同步。

客户端现有数据形态决定了同步的难度分布：

| 数据 | 变更性质 | 同步难度 |
|---|---|---|
| 会话消息（`sessions/<key>.jsonl`） | **纯追加**——编辑、切分支、压缩摘要都是追加新行（ADR-0026/0044） | 低 |
| 错题事件流（`events.jsonl`） | **纯追加**（ADR-0039） | 低 |
| 掌握度调度（`schedule.json`） | **派生状态**——事件流的折叠（ADR-0040） | 无需同步 |
| 会话元数据 / 错题快照 / 记忆 | 可变 | 需要冲突策略 |
| 附件原图（`uploads/`） | 不可变但体积大 | 成本问题 |

好消息是**绝大多数数据是纯追加的**，这决定了冲突面极小；而 ADR-0040 把"事件流为证据、调度为折叠状态"分开的那个决定，在此白捡一个好处：掌握度天然无冲突，服务端永远不需要合并 `interval/ease/due_at`。

## 决策

### 1. 定位：本地是真相源，同步是后台维护的增量副本

三条不可让步的性质：

- 断网时客户端**功能零退化**（只停止上传）
- 网络或服务端故障**不阻塞任何本地写操作**
- 同步失败只意味着"没同步"，不意味着"数据损坏"

一旦为了让同步而让本地写入等待网络，本设计即失效。

### 2. 同步对象

**同步**：会话元数据、会话消息、错题快照、错题事件流、记忆条目。

**不同步**：

- `schedule.json` —— 由事件流折叠重算（ADR-0040），两端都能算，存它只会引入无意义的冲突面
- 附件原图 —— 首期不同步（见决策 9），协议预留端点与字段，以后加不改协议

### 3. 服务端形态：结构化入库

```
sessions(id, user_id, key, title, created_at, last_activity_at, archived_at, deleted_at, rev, updated_at)
session_messages(id, user_id, session_key, message_id, parent_id, kind, payload jsonb, deleted_at)
    unique(user_id, session_key, message_id)
mistakes(id, user_id, mistake_id, subject, knowledge_point, title, question, student_answer,
         reference_answer, analysis, is_correct, pinned, deleted_at, rev, updated_at)
mistake_events(id, user_id, mistake_id, event_id, payload jsonb, created_at)
    unique(user_id, mistake_id, event_id)
memory_entries(id, user_id, path, content, rev, deleted_at, updated_at)
changes(id bigserial, user_id, entity_kind, entity_id, rev, op[upsert|delete], device_id, changed_at)
blobs(sha256, user_id, size, mime, created_at)        -- 首期留空表，不接收上传
```

两个取舍：

- **消息 `payload` 存 jsonb**：服务端不紧跟客户端消息 schema 的演进（消息种类由 kernel 决定），避免每次客户端加字段都要发服务端版本。
- **错题结构化而不是存 JSON blob**：二期"老师看班级学情"就是几条 SQL，不需要再补一次迁移。这是现在值得多花的那点力气。

### 4. 增量机制：服务端 change log + 客户端 outbox

**服务端**：所有写入都往 `changes` 追加一行，`changes.id` 即同步 cursor（单调递增），增量拉取只认它：

```
POST /api/v1/sync/push
  { device_id, ops: [ {kind:"session_meta", key, title, archived_at, rev},
                      {kind:"message", session_key, message_id, parent_id, kind, payload},
                      {kind:"mistake", mistake_id, fields, rev},
                      {kind:"mistake_event", mistake_id, event_id, payload},
                      {kind:"memory", path, content, rev},
                      {kind:"delete", entity_kind, entity_id} ] }
  → { accepted, conflicts: [...], cursor }

GET  /api/v1/sync/pull?since=<cursor>&limit=500
  → { changes: [...], cursor, has_more }
```

**客户端**：storage 在每次成功写入后追加一条 outbox 记录（域内文件 `sync_outbox.jsonl`），推送成功后推进 `synced_seq` 水位：

```json
{"seq": 128, "kind": "message", "session_key": "...", "offset": 40960}
{"seq": 129, "kind": "mistake", "mistake_id": "...", "rev": 4}
```

选择 outbox 而非"读全量再比对"的理由：

- 会话 JSONL **只追加不修改**，字节偏移就是天然单调水位，增量定位是 O(1)，不需要维护"已同步 id 集合"（那会无限增长）
- 错题 `mistake.json` 是覆盖写，无法靠水位判断，只能靠写操作时产出变更记录
- 统一 outbox 让三类数据（会话/错题/记忆）走同一条增量通道，实现与排障都只有一套

主要改动点是 storage 的四个写方法：追加消息、保存错题、追加错题事件、写记忆。**JSONL 文件格式不变**（outbox 是旁路文件），旧数据首启时按全量入队一次。

客户端本地同步状态 `sync_state.json`（域内文件）：`cursor` / `synced_seq` / `device_id` / `last_sync_at`。

### 5. 幂等

消息与事件以自然唯一键 `ON CONFLICT DO NOTHING` 写入（`message_id` / `event_id` 均为 UUID）。因此**重复推送无副作用**，断线重连不必关心"上次推到哪一步"。

### 6. 冲突策略

| 数据 | 策略 |
|---|---|
| 会话消息 / 错题事件 | **并集**，永不冲突（UUID + 唯一键） |
| 会话元数据 / 错题快照 / 记忆 | **LWW by `updated_at`**，服务端为准 |
| 被 LWW 覆盖的一侧 | **不静默丢弃**：落本地 `conflicts/` 目录保留副本，GUI 只显示一个计数 |

不做冲突解决 UI。单人多设备场景下真正的并发编辑概率极低，为它建一套合并界面是过度设计；但**静默丢数据不可接受**，所以保留副本 + 可见计数。

### 7. 删除语义

- 错题：沿用 `deleted_at` 软删（ADR-0038 的"管理状态"语义），同步软删状态，不物理删除
- 会话：删除写 tombstone（`op=delete` 的 change 记录）；客户端本地删除记录保留至推送成功
- 服务端不提供"物理删除"给同步链路；用户主动的"删除云端数据"是账号级操作（见决策 10）

### 8. 触发与重试

三处触发（全自动，无手动按钮）：**回合结束后**（有新数据时）、**启动时**、**每 5 分钟**。失败按退避重试，**静默处理**——只在设置页与左下角显示状态，不弹错误对话框。

RPC 仍提供 `sync_now` 供排障与集成测试使用，但 GUI 不放按钮（自动触发已覆盖正常场景）。

### 9. 附件原图：首期不做，留作独立计费的增值项

首期**不同步 `uploads/` 原图**。在缺少原图的设备上，消息里的附件引用渲染为"图片未同步"占位（可提示在原设备查看）。

协议预留：`AttachmentRef` 携带 `sha256`，服务端预留 `POST /api/v1/blobs`（内容寻址，已存在即秒回）与 `GET /api/v1/blobs/<sha256>`（渲染时懒加载），并预留单图大小上限与用户存储配额。将来开放时无需改动同步主协议。

理由：作业照片是存储与带宽的大头，成本应单独定价，而不是摊进月卡（`plans` 已有承载加量项的位置）。

### 10. 隐私

- `account.sync_enabled` **默认 `false`**；OOBE 登录后询问，设置页可随时关闭
- 关闭后**不再上传**；已上传数据提供「删除云端数据」（`delete_cloud_data`，账号级操作）与全量导出
- 与中转链路严格分离：中转请求/响应正文不落库（ADR-0047 决策 5/11），同步只接收客户端**主动上传**的数据。「同步」是用户显式选择的功能，**不是**服务端对中转流量的顺手记录——这条区别是隐私表述的基石
- 服务端同步相关的所有查询强制带 `user_id`

### 11. 客户端实现位置

新增内核级模块 `src/kernel/sync/`（与 session scheduler 同级，不占 `ServiceId`）：

```
src/kernel/sync/
├── mod.rs      装配、状态机（Idle / Syncing / Error）
├── push.rs     读 outbox → 组装 ops → 推送 → 推进 synced_seq
├── pull.rs     按 cursor 拉取变更 → 交 storage 落盘
└── state.rs    sync_state.json（cursor / synced_seq / device_id）
```

RPC：`sync_now`（排障/测试）、`get_sync_status`、`delete_cloud_data`；事件：`sync_state { phase, pending, last_sync_at, error }`。

拉取到的远端变更由 storage 经既有 DomainIo 通道落盘，**不改客户端 JSONL 格式**。

## 依据

- **冲突面小是既有设计的产物**：消息树追加式 + 事件流追加式（ADR-0026/0039）让绝大部分同步是集合并；`schedule.json` 是折叠状态（ADR-0040）直接免同步。因此 LWW 只需覆盖"元数据 / 错题快照 / 记忆"三类，冲突解决 UI 无必要。
- **change log cursor 而非时间戳**：时间戳在并发写入与时钟漂移下不可靠（同毫秒、回拨），单调整数序列才可靠。
- **outbox 而非全量比对**：错题快照是覆盖写，天生无法用水位判断变更；统一 outbox 是唯一能可靠捕捉全部变更的机制。
- **附件后置**：存储成本应独立定价（用户明确"原图是另外的价钱"），而协议预留让后置不产生返工。

## 影响

- **storage 改动**：四个写方法追加 outbox 记录；客户端 JSONL 格式不变、旧数据兼容（首启按全量入队一次）。这是本 ADR 对既有代码侵入最大的部分，必须保证既有 146 项单测与 live_api 全绿。
- **新增旁路文件**：`sync_outbox.jsonl`、`sync_state.json`、`conflicts/` 三个域内路径，需纳入数据根目录结构与 `RelocPath`/`DomainIo` 的域枚举。
- **服务端新增**：`src/sync/`（push/pull/cursor/冲突裁决）与六个同步实体表。
- **隐私文案**：`docs/usage.md` 与用户协议需明确"中转不落正文 / 同步需主动开启"的区别。
- **未验证项**：双设备收敛一致性（需真实两端测试）；大会话（数万条消息）的增量推送与首次全量入队耗时；LWW 覆盖时的本地 `conflicts/` 可恢复性；断网期间 outbox 增长上限。
- **文档同步**：`PROJECT.md`（服务端与同步章节）、`CONTEXT.md`（Sync cursor / Sync outbox / Device sync / Blob store 术语）、`docs/TODO.md`（里程碑）。

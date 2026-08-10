# scheduler 内核插件、磁盘 IO 铁律与数据运行时化

决策：场景 5（长效追踪，ADR-0041）落地的三条机制层决策，均超出 0041 原范围，单独留痕：

## 1. scheduler 内核插件（定时中断模型）

新增 `ServiceId::Scheduler` + `SchedulerHandle`。用户插件（tracking）经句柄注册定时配置（`interval` + 提醒载荷文本 + `fire_on_start`）；scheduler 只存配置、到点**请求 kernel 核心**，由内核特权在回合边界发起 `Interrupt::Timer`——用户态只发系统调用，用户插件永不触碰中断（OS 式系统调用/特权中断分层）。kernel 核心维护 pending proactive 回合队列（与 `send_user_message` 并列的回合触发源），回合空闲时消费、不并入用户回合；proactive 回合工具白名单缺省为空 + 全局频率硬护栏（防失控插件烧钱，白名单只缩不扩）。

动机：定时器/主动回合写进 kernel 核心违反"功能由插件扩展"原则——storage/memory/compute/model 均为内核插件，kernel 核心无硬编码能力先例；"发中断"是特权动作，用户态直接触达破坏信任模型；subscribe 周期回调方案让 scheduler 持有业务回调注册表，机制与策略边界糊。

## 2. 磁盘 IO 铁律

用户插件的一切磁盘读写（错题、图谱、真题池、依赖表、调度）只经 StorageHandle，不持有任何文件句柄；内核插件各自管理自己的持久化目录不变（memory/ 归 memory、audit/logs 归 storage）。

### 落地形态（双 trait + 类型安全路径）

- **`DomainIo`**（`plugin/services/storage.rs`）：数据根目录域内文件能力（`read/write/remove/remove_tree/list`，域 = `Domain` 枚举：mistakes/sessions/memory/data/uploads）；实现（storage）内部负责域根拼接 + `dunce::canonicalize` 兜底（防符号链接逃逸、Windows `\\?\` verbatim）+ 原子写（tmp + rename）+ 审计（`AuditRecord::FileIo`）。用户插件永不持有本 trait——只见 `StorageHandle` 语义方法（`read_staged/remove_staged/read_data_file/write_data_file`，均记审计）。
- **`TmpIo`**：系统 temp 暂存文件能力（`read_staged/remove_staged`），硬编码 `std::env::temp_dir()` + `mistake-agent-` 前缀白名单，与 DomainIo 解耦、不做目录管理；读删记 `AuditRecord::StagedFileIo`。附件暂存（vision 读、grading 删）从插件直读文件改为经本 trait（原 `stage_path_allowed` 白名单逻辑搬入实现，唯一实现点）。
- **`RelPath`**（类型安全路径）：构造即校验——段必须以 `[a-zA-Z0-9]` 开头结尾、中间仅 `[a-zA-Z0-9._-]`；空段、`.`、`..`、尾点/首点、`\`、`:`、非 ASCII（同形字符攻击面）全部拒绝。**不做任何路径语义解析/规范化**（规范化即攻击面），fail-closed：parse 失败即调用失败。类型上不可能表示目录遍历。
- **用户插件零文件句柄**：vision/grading/practice 全部经 StorageHandle 语义方法；practice 真题池改运行时（见 §3）；main.rs GUI 壳非插件、保留自有 canonicalize 白名单。
- **存储布局迁移**：memory 旧布局（中文路径直接落盘 `memory/测试/记忆条目.md`）→ 新布局（base64url 段编码）由 `FileMemoryService::migrate_legacy_layout` 在 Kernel 启动时迁移（幂等：可解码的新布局条目跳过；旧条目读入→编码写出→删旧文件，失败不阻塞启动）。迁移通道 = `DomainIo::read_legacy/remove_legacy`（允许非 ASCII 段但拒绝 `..`/`\`/绝对路径/空段，canonicalize 兜底，审计照记）——宽松不等于不校验，仅引导迁移使用。

### 威胁模型声明

防护目标是**防路径参数注入（插件 bug / 恶意插件）**，不是防本地恶意进程——能往数据根目录写符号链接的进程已拥有读取全部文件的权限，绕过 storage 校验无意义。残余 TOCTOU 窗口（校验后、open 前符号链接被换）作为接受的风险；open 统一用 canonicalize 结果、写入统一 tmp + rename 已消减大部分窗口。

## 3. 数据运行时化

教学数据（真题池 `data/gaokao_pool.json`、先验依赖表 `data/point_deps.json`）从编译期 include_str! 改为数据根目录 `data/` 运行时读取；bootstrap 种子写入（缺失时写默认，与 AGENTS.md 模板同款幂等逻辑）；数据可编辑、可被模型经 storage 句柄更新（生成即数据）。practice 真题池读取连坐改运行时（已验收代码翻案）。

### 落地形态

- `bootstrap.rs` 子目录加 `data/`。
- `exam_pool.rs`：`include_str!` 常量改名 `DEFAULT_POOL_JSON`（**内置种子兜底**）；`read_pool_json(storage)` 经 `StorageHandle::read_data_file("gaokao_pool.json")` 读运行时文件，缺失/损坏（含解析失败）回退种子，不报错（出题不因数据问题崩）；`draw_from_pool` 改纯函数（接收 pool JSON 参数，测试友好）。
- `practice::generate` 的 Exam 分支读运行时文件；`templates::build_item` 同步路径用种子兜底。
- 数据文件路径经 `RelPath` 白名单（`data/` 域内），原子写。

动机：编译期写死数据 = 改数据要发版；堵死"模型生成数据落盘"路径；AGENTS.md 已是运行时数据的正确先例。

考虑过的替代方案：subscribe 周期回调（被否：scheduler 持有业务回调，见上）；scheduler 直接 send 中断（被否：中断只由内核特权发起）；数据编译期嵌入（被否：更新要发版、模型无法生成数据）。

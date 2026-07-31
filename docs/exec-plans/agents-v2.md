# Agents v2 —— 从「两套系统」到「一套 agent 操作系统」

> 状态：**提案 v2**（2026-08-01，按 owner 反馈修订：消息底座全走 CLI 不走 MCP、hooks 作为遥测通道、server 可选原则、agent home 隔离原则）。
> 配套原型：[`agents-v2-prototype.html`](agents-v2-prototype.html)（桌面三栏布局，浏览器直接打开）。
> 前序文档：[`projects-and-tasks.md`](projects-and-tasks.md)（P0 已落地，本文取代其 P1+ 规划）。

## 0. 一句话

把 Projects（声明式工作区）和 Team（多 agent 群聊）合并成一个系统：**项目是舞台，agent 是演员，`tmm` CLI 是他们唯一的说话方式，hooks 是我们观察他们的眼睛，tmux 窗口是他们的工位**——中心化定义 agent（backend + skills + MCP + 人设，装进隔离的 home），在任何项目里拉起，lead agent 可以再拉起别人；server 挂了他们照样干活。

## 1. 五条设计原则

前三条继承已验证的经验，后两条是本次修订确立的（都在 Team 编排里实践过，现在升格为原则）：

1. **声明是真相，tmux 是投影**（P0 原则，扩展到 agent：registry 声明「agent 是什么」，slot 声明「agent 在哪」，tmux 窗口是运行态投影）。
2. **CLI 是 agent 唯一的主动接口**。收发消息、汇报状态、拉起同伴——全部是 `tmm` 子命令。**不再用 MCP 工具做消息底座**：MCP 要求每个 backend 物化配置 + 协议握手 + 三套写法（kiro/claude/codex 各不同），CLI 只需要 system prompt 里一行「用 `tmm send` 说话」。agora 的 MCP daemon 随 Team legacy 一起退役，不迁移。
3. **agent 的一切行为可见**：tmux 窗口看得到、`tmm log` 查得到、UI 中枢读得到。没有藏在后台的执行。
4. **Server 可选，永不阻塞**。agent 系统脱离 server 也能跑：agent 本质是 tmux 窗口里的普通 CLI 进程，server 只是「工具 + hooks」的提供方。`tmm` 连不上 server 时一句话报错退出（exit 2），hooks 脚本超时静默放弃——**任何一环失败都不能卡住 agent 的正常工作**。你随时可以 kill server，窗口里的 agent 毫无感觉。
5. **Agent home 隔离**。每个 agent 拿到一个独立的 HOME（`KIRO_CONFIG`/`CODEX_HOME`/claude settings dir——Team 的 `prepare_kiro_home` 已实践），skills、MCP、hooks、人设全部由 registry 定义写入这个 home，**用户空间的全局配置零干扰**。同一个 registry 定义在任何项目、任何机器上行为一致。

## 2. 从 Multica 学什么、不学什么

实际使用 + 源码/文档研究（CLI_AND_DAEMON.md、内置 squads skill）得出的结论：

### 学（概念）

| Multica 概念 | 借鉴点 | 我们的对应物 |
|---|---|---|
| **Agent 定义是 workspace 资产** | agent = 名字 + backend CLI + model + skills + mcp_config，中心化定义、处处可用 | 新增 **Agent Registry**（现在 agent 定义埋在 team 模板 YAML 里，一次性、不可复用） |
| **Skills / MCP 中心化配置、按 agent 赋予** | skills 独立资产挂到 agent；per-agent `mcp_config` 物化进隔离环境 | 我们已有 skills 解析（本地 + GitHub sparse-clone）和 MCP 物化（`launch.rs`/`backends.rs`）——把它们从模板里提出来变成一等资产 |
| **Squad 只路由，不扇出** | 一切工作路由到 `leader_id`，由 leader 按成员能力主动委派 | 「lead agent 分配」模式；我们的 hire/fire 已是这个形状 |
| **CLI 设计范式** | 见下方专节 | `tmm` CLI |

### Multica CLI 值得抄的作业（专节）

逐条读了 `multica` CLI 的文档，这些约定直接采用：

- **`--output json` 全覆盖**：每个读命令都有机器可读输出，agent 和脚本才能可靠消费。
- **分层 exit code**：`0` 成功 / `2` 网络 / `3` 鉴权 / `4` 不存在 / `5` 参数校验——脚本（和 agent！）能按错误类别分支，不用 parse 错误文本。
- **错误一句话可行动**：超时提示查网络、401 提示重新配 token；`--debug` 才吐完整错误链。默认英文，检测到中文 locale 自动切中文。
- **增量拉取游标**：`--since <ts>` / `run-messages --since <seq>`——轮询消息流不重复搬运，对 agent 的上下文窗口友好（它的 comment 分页设计整个就是为「别把几百条回复拖进 prompt」服务的）。
- **短 ID + `--full-id`**：表格显示短前缀，需要时拿全 UUID。
- **`--help` 优先于猜**：它的内置 skill 反复强调「命令形状不确定就先 `--help`」——我们的 system prompt 注入也这么写。

不抄的：auth/login 浏览器流（我们是单机 token）、workspace/profile 多租户、daemon 轮询模型（我们的 agent 在 tmux 里，不需要 daemon 代跑）。

### 不学（架构）

- **不学 Go+Postgres+Next.js 三件套**：我们是单 Rust 进程 + SQLite + Svelte。
- **不学 headless 隔离 scratch dir**：agent 藏在 daemon 后台是 Multica 的选择；我们每个 agent 一个 tmux 窗口、在真实项目目录工作。可见性是差异化，不是妥协。
- **不学 Agent 看板 / issue tracker 全家桶**（backlog/priority/due-date…）：不做看板，但**状态要追踪**——见 §4.3 状态模型。
- **不学云端账户体系**。

## 3. 现状盘点（我们已经有什么）

| 资产 | 位置 | v2 中的命运 |
|---|---|---|
| Projects P0：声明式项目、auto-adopt、agent slot resume、tmux 投影 | `src-tauri/src/projects/`, `state.db` | **保留，成为唯一的「工作发生地」** |
| agora bus：房间、信封、SQLite 存储 | `src-tauri/crates/agora/` | **bus 存储保留为消息底座**；其 **MCP daemon 不再是 agent 接口**，随 Team legacy 退役 |
| Team：模板 YAML、skills 解析、MCP/env 物化、**per-agent home 隔离**、hire/fire、hook 布线（pre/postToolUse/stop/notification 全套） | `src-tauri/src/team/` | **拆解重组**：backends.rs 的接入层 + workspace.rs 的 home 隔离 + skills.rs 上移为共享服务 |
| 通知 hub：hook 驱动的 agent 状态/session_id | `agent_notifications.rs` | **扩展为遥测管道**（§4.2），继续喂 resume 和 attention dot |
| 桌面 UI：底部 tab | `src/` | **桌面改三栏**，移动端保持 tab |

关键判断不变：**v2 是合并不是重写**。home 隔离、hook 布线、skills/MCP 物化全部已在 `team/backends.rs` 实践过，v2 把它们从「Team 专属」提炼成「所有 agent 的接入层」。

## 4. 目标架构

```
┌──────────────────────────────────────────────────────────┐
│  tmux-mobile server（单进程，可选 —— 挂了 agent 照常干活）  │
│                                                           │
│  Agent Registry ── 定义: backend+skills+MCP+人设           │
│       │ 物化到隔离 home（KIRO_CONFIG/CODEX_HOME/…）         │
│  Project（state.db）── agent slot（窗口即工位）             │
│       │ 投影                                               │
│  tmux session ── 每个 agent 一个可见窗口（普通 CLI 进程）    │
│       │                                                   │
│  agora bus（per-project room）── 消息 / 状态 / 遥测事件      │
│       ↑ 主动通道         ↑ 被动通道        ↑                │
│    tmm CLI            hooks 遥测        WS RPC            │
│   （agent 说话）      （我们观察）       （手机/桌面 UI）      │
└──────────────────────────────────────────────────────────┘
```

### 4.1 双通道：agent 说的 vs 我们看到的

owner 定的关键切分——**主动内容走 CLI，过程遥测走 hooks**：

- **主动通道（`tmm`，agent 有意说的话）**：最终回复、提问、状态宣告、拉起同伴。这些必须是 agent 的自觉行为，写进 system prompt。
- **被动通道（hooks，我们观察到的事实）**：`preToolUse`/`postToolUse` → 正在调什么工具；`userPromptSubmit` → 收到了输入；`stop`/`notification` → 停下来了/要权限/要输入。Team 已布线全套，v2 把 payload 汇入 bus 成为遥测事件流。
- 两路互补去重：hooks 说「它停了」+ 没有 `tmm done` = **可能卡住**（stuck 判定）；hooks 说「在调工具」= UI 状态卡片上的实时活动行，agent 不用自己汇报「我在干活」。

### 4.2 接入层（per-backend adapter）

「维护每一个 coding agent 的接入」= 一张表回答四个问题（现状分散在 `team/backends.rs` + `projects/agents.rs`，v2 合并成一个 adapter 层）：

| 问题 | 内容 |
|---|---|
| 怎么启动 | 命令行 + args + env（含隔离 home 变量名：kiro=`KIRO_CONFIG`、codex=`CODEX_HOME`、claude=settings dir…） |
| 怎么注入 | skills/MCP/人设/hooks 写进隔离 home 的哪个文件、什么格式（三套写法已在 backends.rs 实现） |
| 怎么观察 | 该 backend 支持哪些 hook 事件、payload 里有什么（kiro/claude 全套，codex 部分——差异记录在表里，缺的 hook 用 tmux pane 活动兜底） |
| 怎么恢复 | resume 方式（P0 已解决：`--resume-id`/`--resume <id>`/`codex resume <id>`） |

新接一个 coding agent = 在这张表加一行 + 一个 home 模板，不碰其他代码。

### 4.3 状态模型（追踪但不看板）

状态是**推导**出来的，不是 agent 填表：

```
working   ← hooks 有工具活动（30s 内有 pre/postToolUse）
waiting   ← 显式 tmm status waiting，或 notification: 要权限/要输入
idle      ← stop 且已 tmm done
stuck     ← stop 但没有 done，且 N 分钟无活动（Team 的 keepalive 判定复用）
offline   ← tmux 窗口没了
```

每个 agent 的追踪记录 = 状态 + 最近活动行（最后一个工具调用/最后一条消息）+ 时间戳。呈现在三处：UI 状态卡片、sidebar 状态点、`tmm agent list`。不做看板列、不做拖拽、不做 priority。

### 4.4 `tmm` CLI（唯一的主动接口）

新 cargo bin（`src-tauri/src/bin/tmm.rs`），连本机 server 的 WS RPC（token 从 config.toml 读）。**fail-soft 是硬性约定**：server 不在 → 一句话 + exit 2，绝不重试挂起，绝不阻塞调用方。

```bash
# agent 用（launch 时 export TMM_PROJECT / TMM_AGENT，system prompt 注入一行用法）
tmm send "@reviewer 看一下 src/lib.rs 的改动"   # 说话（最终回复也走这里）
tmm log --since <ts> [--output json]           # 拉新消息（增量游标，学 multica）
tmm status waiting "等接口定稿"                  # 显式状态宣告
tmm spawn reviewer --brief "review 当前分支"     # lead 拉起同伴（can_hire 门控）
tmm done "PR 已提交"                            # 完成宣告

# 人用
tmm agent list [--all] [--output json]  # 谁在干活、什么状态、最近在干嘛
tmm log -f                              # 跟随消息流
tmm project list / tmm registry list
```

- 约定照抄 multica：`--output json` 全覆盖、分层 exit code（0/2/3/4/5）、错误一句话可行动、短 ID、增量 `--since`。
- **没有 `tmm wait` 长轮询了**：v1 提案的开放问题被 owner 的双通道切分直接解决——agent 不守株待兔，「有新消息」由 hooks/notification 唤醒（或 lead 主动 `tmm log --since`），拉取是瞬时命令。

### 4.5 Agent Registry（中心化定义 + 隔离 home）

```
agent 定义 = {
  name, backend, model?, system,      # 人设
  skills: [引用], mcp: [引用],         # 一等资产，按引用组合
  can_hire: bool,                     # lead 权限
}
```

- 存储：`state.db` 新表 agents / skills / mcp_servers（迁移沿用 FK-off 教训）。
- **物化 = 写隔离 home**：拉起时把定义渲染进该 backend 的隔离 home（adapter 层负责格式），用户全局配置零干扰。改 registry 里的一个 skill，下次拉起所有引用它的 agent 生效。
- 内置种子：从 team 模板提炼 lead/reviewer/docs 等预设。

### 4.6 任务分配：lead agent 模式

用户场景直接映射（不变）：

- **(a) 布置任务**：项目里 `+ agent` 拉起 lead，人在它的窗口（或 UI 中枢 @它）布置任务——对话就是任务，没有 issue 表单。
- **(b) 动态拉起**：lead 需要帮手 → `tmm spawn reviewer --brief "..."` → 同一项目 session 开新窗口、按 registry 物化、brief 作为开场消息。
- **(c) 状态监控**：§4.3 的推导状态 + 遥测活动行，三处可见。

第一版不做 leader briefing 自动名册注入——lead 用 `tmm agent list` / `tmm registry list` 主动查，把判断留给 agent。观察效果后再定。

### 4.7 UI：桌面三栏（原型见配套 HTML）

不变：左 Sidebar（项目 → agent/窗口树 + registry）/ 中交互中枢（群聊流 + 状态卡片）/ 右 Terminal（一键全屏）。移动端零改动。中枢的状态卡片直接消费 §4.3 的推导状态和遥测活动行。

## 5. 分阶段执行

### Phase A —— `tmm` CLI + 遥测管道（地基）
- 新 bin `tmm`：send / log(--since/-f) / status / done / agent list / project list。fail-soft + exit code 分层 + `--output json`。
- bus room 与 project 关联（room id = project session）。
- hook 遥测汇入 bus：扩展 `agent_notifications.rs` 管道，pre/postToolUse payload 入 room 事件流；§4.3 状态推导。
- 验证：真实 kiro agent 在项目窗口用 `tmm send` 对话；kill server，agent 正常继续干活，`tmm` 报 exit 2。
- 交付物：CLI + `docs/design-docs/features/tmm-cli.md`。

### Phase B —— Agent Registry + 接入层
- state.db 新表；backends.rs/workspace.rs/skills.rs 的物化+隔离逻辑上移为共享 adapter 层（§4.2 的表）。
- RPC registry CRUD + 最简管理 UI；内置种子 agent。
- 验证：registry 定义带 skill+MCP 的 agent，在两个项目拉起行为一致；宿主机用户全局配置改动不影响 agent 行为（隔离验证）。

### Phase C —— 项目里的 agent（融合核心）
- 项目 `+ agent`、`tmm spawn`（can_hire + per-project 上限默认 4）、状态/遥测全链路。
- Team 页标记 legacy；`tmm-team-` 会话继续工作不迁移。
- 验证：完整走一遍场景 (a)(b)(c)；宕机重启后 agent resume（P0 机制）+ 状态恢复。

### Phase D —— 桌面三栏 UI
- 按原型实施；移动端零改动验证（Android APK 回归）。

### 排除项（明确不做）
- 看板/优先级/截止日期。
- MCP 作为 agent 接口（bus 的 MCP daemon 随 Team 退役；registry 里的 MCP 是 agent 自己的工具，不是我们的消息通道——两回事）。
- autopilot/cron（v2 稳定后单独评估）。
- 移动端三栏。
- agora vendored crate 改动（room 映射在我们这层做）。

## 6. 风险与开放问题

1. **codex 的 hook 覆盖不全**：遥测通道在 codex 上退化为 tmux pane 活动兜底（adapter 表里记录差异）。状态推导要能容忍「只有 pane 活动、没有工具粒度」的 backend。
2. **Team→Projects 迁移节奏**：C 阶段并行两周后决定是否下线 Team 页（连同 agora MCP daemon）。
3. **spawn 资源失控**：per-project 上限默认 4，超限报错让 lead 收敛。
4. **hooks 写 bus 的时序**：hook 脚本必须 fire-and-forget（curl --max-time 2 级别），宁可丢遥测也不能拖慢 agent——原则 4 在实现层的落点。

# Agents v2 —— 从「两套系统」到「一套 agent 操作系统」

> 状态：**提案**（2026-08-01），供 review。定稿后拆分为分阶段 exec-plan 执行。
> 配套原型：[`agents-v2-prototype.html`](agents-v2-prototype.html)（桌面三栏布局，浏览器直接打开）。
> 前序文档：[`projects-and-tasks.md`](projects-and-tasks.md)（P0 已落地，P1 尚未开始，本文取代其 P1+ 规划）。

## 0. 一句话

把 Projects（声明式工作区）和 Team（多 agent 群聊）合并成一个系统：**项目是舞台，agent 是演员，`tmm` CLI 是他们说话的方式，tmux 窗口是他们的工位**——中心化定义 agent（backend + skills + MCP + 人设），在任何项目里拉起，lead agent 可以再拉起别人，一切在 tmux 里可见、在 UI 里可管、在 CLI 里可查。

## 1. 从 Multica 学什么、不学什么

实际使用 + 源码/文档研究（CLI_AND_DAEMON.md、内置 squads skill）得出的结论：

### 学（概念）

| Multica 概念 | 借鉴点 | 我们的对应物 |
|---|---|---|
| **Agent 定义是 workspace 资产** | agent = 名字 + backend CLI + model + skills + mcp_config，中心化定义、处处可用 | 新增 **Agent Registry**（现在 agent 定义埋在 team 模板 YAML 里，一次性、不可复用） |
| **Skills / MCP 中心化配置、按 agent 赋予** | skills 是独立资产，挂到 agent 上；per-agent `mcp_config` 由 daemon 物化成临时配置传给 CLI | 我们已有 skills 解析（本地 + GitHub sparse-clone）和 MCP 注入（`launch.rs` Extras）——只差把它们从模板里提出来变成一等资产 |
| **Squad 只路由，不扇出** | 一切工作路由到 `leader_id`，leader briefing 里带成员名册 + 各自 skills，**由 leader 按能力委派** | 这正是用户要的「lead agent 分配」模式，而且我们的 hire/fire 已经是这个形状（manager only） |
| **CLI 是 agent 的手** | agent 通过 CLI 收发消息、改状态、查上下文；`--output json` + 分层 exit code | 新增 **`tmm` CLI**（取代「hooks + MCP」这套注入复杂度） |
| **一切可查** | issue runs / run-messages / usage，增量 `--since` | `tmm agent list` / `tmm log` 等 |

### 不学（架构）

- **不学 Go+Postgres+Next.js 三件套**：我们是单 Rust 进程 + SQLite + Svelte，够用且已有。
- **不学 headless 隔离 scratch dir**：Multica 把 agent 藏在 daemon 后台、每任务开隔离目录；用户明确要**每个 agent 一个 tmux 窗口**、在真实项目目录工作。可见性是我们的差异化，不是妥协。
- **不学 issue tracker 全家桶**（backlog/priority/due-date/position…）：我们不是 Linear。任务粒度上一版已定：短生命周期执行单元。
- **不学云端账户体系**：单机、token 授权，现状足够。

## 2. 现状盘点（我们已经有什么）

| 资产 | 位置 | v2 中的命运 |
|---|---|---|
| Projects P0：声明式项目、auto-adopt、agent slot resume、tmux 投影 | `src-tauri/src/projects/`, `state.db` | **保留，成为唯一的「工作发生地」** |
| agora bus：房间、信封、SQLite 存储 | `src-tauri/crates/agora/` | **保留为消息底座**（不动 vendored crate） |
| Team：模板 YAML、skills 解析、MCP/env 注入、hire/fire、状态跟踪 | `src-tauri/src/team/` | **拆解重组**：skills/MCP 注入逻辑上移为 registry 服务；`tmm-team-` 独立会话模式逐步并入项目 |
| MCP daemon (:8787)：send_message/wait/list_agents/read_history/hire/fire | `agora/src/mcp.rs` | **保留但降级为兼容层**；`tmm` CLI 成为首选交互方式 |
| 通知 hub：hook 驱动的 agent 状态/session_id | `agent_notifications.rs` | **保留**，继续喂 resume 和 attention dot |
| 桌面 UI：底部 tab（Sessions/Terminal/Team/Files） | `src/` | **桌面改三栏**，移动端保持 tab |

关键判断：**v2 不是重写，是合并**。两套系统各自缺的恰好是对方有的——Projects 有「地方」没「人」，Team 有「人」没「地方」。

## 3. 目标架构

```
┌─────────────────────────────────────────────────────────┐
│  tmux-mobile server（单进程）                              │
│                                                          │
│  Agent Registry ──── skills / MCP / 人设 中心化定义        │
│       │ 实例化                                            │
│  Project（state.db）─── agent slot（窗口即工位）           │
│       │ 投影                                              │
│  tmux session ──── 每个 agent 一个可见窗口                 │
│       │                                                  │
│  agora bus（per-project room）─── 消息 / 状态 / 事件       │
│       ↑           ↑                ↑                     │
│    tmm CLI     MCP daemon       WS RPC                   │
│   （agent 用） （兼容层）        （手机/桌面 UI 用）         │
└─────────────────────────────────────────────────────────┘
```

三条设计原则（继承已验证的经验）：

1. **声明是真相，tmux 是投影**（P0 原则，扩展到 agent：registry 声明「agent 是什么」，slot 声明「agent 在哪」，tmux 窗口是运行态投影）。
2. **一个概念一个入口**（P0 的「一个 +」原则：agent 只有一种定义方式 registry，只有一种交互方式 bus，CLI/MCP/UI 都是 bus 的不同门面）。
3. **agent 的一切行为可见**：tmux 窗口看得到、`tmm log` 查得到、UI 中枢面板读得到。没有藏在后台的执行。

### 3.1 Agent Registry（中心化 agent 定义）

```
agent 定义 = {
  name:        "reviewer",
  backend:     kiro | claude | codex | kimi | ...,
  model:       可选覆盖,
  system:      人设/职责 prompt,
  skills:      [skill 引用],     ← 复用 team/skills.rs 的解析（本地 + GitHub）
  mcp:         [MCP server 定义], ← 复用 launch.rs 的物化注入
  can_hire:    bool,             ← lead 权限：能否拉起别的 agent
}
```

- 存储：`state.db` 新表（和项目同库——agent 定义是本机资产，不是某个 team 的私产）。
- Skills 和 MCP server 各自也是一等资产（独立表），agent 通过引用组合——改一个 skill，所有引用它的 agent 生效。这是 Multica「中心化配置、按 agent 赋予」的直接对应。
- 内置种子：从现有 team 模板里把 manager/worker/reviewer 等角色提炼成 registry 预设，模板退化为「一组 agent 引用 + 分工说明」。

### 3.2 `tmm` CLI（agent 的手，也是极客的手）

新 cargo bin（`src-tauri/src/bin/tmm.rs`），通过 WS RPC 连本机 server（token 从 config.toml 读，环境变量可覆盖）。两类用户，一套命令：

```bash
# agent 在自己的 tmux 窗口里用（注入方式：launch 时 export TMM_PROJECT/TMM_AGENT）
tmm send "@reviewer 看一下 src/lib.rs 的改动"      # 发消息
tmm wait                                          # 阻塞等新消息（长轮询）
tmm status working "正在重构 store 层"              # 汇报状态
tmm spawn reviewer --brief "review 当前分支"        # lead 拉起一个 agent（need can_hire）
tmm done "PR 已提交"                               # 汇报完成

# 人在任何终端里用
tmm agent list                    # 本项目有哪些 agent、什么状态
tmm agent list --all              # 所有项目
tmm log [-f] [--since <ts>]       # 群聊消息流（-f 跟随）
tmm project list                  # 项目一览
tmm registry list                 # registry 里定义了哪些 agent
```

- 约定学 Multica：`--output json` 全覆盖、分层 exit code（0 成功 / 2 网络 / 3 鉴权 / 4 不存在 / 5 参数）、错误一句话可行动。
- **为什么 CLI 优于 hooks+MCP 注入**：MCP 要求每个 backend 配置文件物化 + 协议握手 + 工具白名单差异（kiro/claude/codex 三套写法），hooks 只覆盖生命周期事件；CLI 是所有 backend 天然都会用的东西——写进 system prompt 一行「用 `tmm send` 说话」就完成注入，零配置文件。MCP daemon 保留给已经配好的存量场景 + 外部 agent，不再扩展。

### 3.3 任务分配：lead agent 模式（学 squad 路由）

用户场景直接映射：

- **(a) 创建项目并布置任务**：项目里 `+ agent` 选一个 registry 定义拉起（默认给一个 lead），人在它的 tmux 窗口里直接布置任务——不需要 issue 表单，对话就是任务。
- **(b) 交互式执行与动态拉起**：lead 干活中发现需要帮手 → `tmm spawn reviewer --brief "..."` → server 在**同一个项目 session 里开新 tmux 窗口**、按 registry 定义注入 skills/MCP/人设、brief 作为开场消息发进去。这就是现有 hire 的形状，但从「Team 专属」变成「任何项目可用」。
- **(c) 状态监控**：`tmm status` 写 bus → UI 中枢和 `tmm agent list` 都能看到；hook 通知（已有）继续提供「agent 停下来了」的被动信号，两路互补。

和 Multica 的差异（有意的）：**不做 leader briefing 自动注入名册**的第一版——我们的 lead 用 `tmm agent list` / `tmm registry list` 主动查，把「委派给谁」的判断留给 agent 而不是 prompt 工程。观察实际效果后再决定要不要 briefing。

### 3.4 UI：桌面三栏（原型见配套 HTML）

```
┌──────────┬──────────────────────┬──────────────────────┐
│ Sidebar  │  交互中枢 (Hub)        │  Terminal            │
│          │                      │                      │
│ 项目列表   │  当前项目的群聊流       │  当前选中 agent/窗口   │
│  └ agent │  （人和 agent 都在说）  │  的真实 tmux pane     │
│    窗口树  │  agent 状态卡片        │                      │
│ 通知      │  输入框 @agent         │  随时全屏切换 ⇄        │
└──────────┴──────────────────────┴──────────────────────┘
```

- **左栏 Sidebar**：项目 → agent/窗口树（复用 Projects 数据 + 窗口 chips），attention dot、agent 状态色点；底部通知和设置入口。
- **中栏交互中枢**：per-project 群聊（bus room 的时间线）+ agent 状态卡片。人在这里 @agent 布置任务、看回复——**不用进 terminal 也能指挥**。复用 Team 页的消息流组件。
- **右栏 Terminal**：保持现状的 xterm 组件，显示当前选中窗口。一键全屏（隐藏左中栏）满足极客需求；移动端布局完全不动（tab 导航保留）。
- 渐进式：三栏是桌面 `isMobile == false` 时的布局重排，组件全部复用，不 fork 代码路径。

## 4. 分阶段执行

依赖关系：CLI 和 Registry 是地基，融合和 UI 在其上。每阶段独立可交付、可验证。

### Phase A —— `tmm` CLI + bus 打通（地基）
- 新 bin `tmm`，实现 send/wait/log/status/agent list/project list。
- bus room 与 project 关联（room id = project session，替代现在的 workspace+template slug）。
- agent launch 时 export `TMM_PROJECT` / `TMM_AGENT`，system prompt 注入一行 CLI 用法。
- 验证：真实 kiro agent 在项目窗口里用 `tmm send`/`tmm wait` 和人对话。
- 交付物：CLI + `docs/design-docs/features/tmm-cli.md`。

### Phase B —— Agent Registry
- `state.db` 新表：agents / skills / mcp_servers（迁移沿用 FK-off 教训）。
- skills.rs / launch.rs 的解析注入逻辑从 team 模块上移到共享层。
- RPC：registry CRUD；UI 先给最简管理页（列表 + 编辑表单）。
- 内置种子 agent（从 team 模板提炼）。
- 验证：registry 定义一个带 skill+MCP 的 agent，在两个不同项目拉起，行为一致。

### Phase C —— 项目里的 agent（融合核心）
- 项目 `+ agent`：从 registry 选定义 → 项目 session 开窗口 → 注入 → 记入 slot（agent slot 已有 resume 机制，直接受益）。
- `tmm spawn`（can_hire 门控）+ `tmm done`；状态流入 bus。
- Team 页保留但标记 legacy；`tmm-team-` 会话继续工作不迁移（老功能不破坏）。
- 验证：完整走一遍用户场景 (a)(b)(c)。

### Phase D —— 桌面三栏 UI
- 按原型实施：Sidebar / Hub / Terminal 三栏 + 全屏切换。
- 移动端零改动验证（Android APK 回归）。

### 排除项（明确不做）
- 看板/优先级/截止日期等 issue tracker 功能。
- autopilot/cron（原 P3，等 v2 稳定后单独评估）。
- 移动端三栏。
- agora vendored crate 的改动（bus 够用，room 映射在我们这层做）。

## 5. 风险与开放问题

1. **`tmm wait` 长轮询 vs agent CLI 的工具调用模型**：交互式 agent（非 headless）会不会真的去轮询？Phase A 用真实 kiro 验证，不行就回退到「hook 通知唤醒 + `tmm log --since` 拉取」。
2. **Team→Projects 迁移节奏**：存量 team 用户（=你）何时切换？计划 C 阶段并行两周后决定是否下线 Team 页。
3. **spawn 的资源失控**：lead 无限拉 agent？第一版给 per-project 上限（默认 4），超限 spawn 报错让 lead 收敛。
4. **三栏最小宽度**：< 1100px 时中栏和 terminal 二选一（原型里有此断点的处理）。

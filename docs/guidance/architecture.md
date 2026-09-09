# Guidance · 架构与边界 · Architecture & boundaries

> 信条：二（壳）、四（无中心）、七（声明是真相）、八（推导不自报）。
> 审这个维度的人问的是：**这段代码是否越出了壳的边界？真相在哪里？服务器死了会怎样？**
> 草案 · 2026-09-09 · 中文讨论稿，定稿后英文化。

## 1 · 原则（信条的展开）

1. **壳的四样东西**：连接、房间、身份、窗口。代码新增的能力若不属于这四样，先找底层
   工具的原生入口（env、配置文件、hooks、启动参数）；找不到时在 design doc 写明为何。
2. **真相只有两处**：state.db 与 `<ws>/.tmm/`。tmux 窗口名、pane 文本、进程、内存缓存
   都是投影，随时可丢、随时可重建。
3. **服务器是观察者**：任何流程若在服务器进程死亡后让人或 agent 卡住，就是设计错误。
4. **一个概念一个定义函数**：`managed_home`、`detect_pane`、`valid_name`、`SPAWNABLE_BACKENDS`
   这类判定只存在一份，所有门共用。
5. **后端差异封在一处**：五个 CLI 的配置格式、hooks 方言、状态行不同，各一份实现是必要的；
   但"加一个后端"只能新增一个后端文件，下游不多一个分支。
6. **外部系统按文档写**：tmux、各 CLI、SQLite 都有版本与解析规则；格式串、目标名、
   迁移语义都要查文档并把版本写进注释。

## 2 · 必须 / 禁止

**必须**
- 幂等：`up`、restart、refresh、迁移都可重复执行而结果不变。
- 先记录再行动：`launch.json` 在窗口创建前写入，写失败即 spawn 失败。
- 删除顺序：先成功 `down`，再删 home，再删行；忽略 `down` 错误会留下无主会话被 `auto_adopt` 复活。
- tmux 目标用精确形式 `=name:`（`-t name` 是前缀/glob 匹配）；分隔符按 tmux 版本的转义规则。
- 同步 I/O（rusqlite、tmux 子进程、sleep）跑在 `spawn_blocking`；`std::Mutex` guard 不跨 `await`；不持 store 锁去观察 tmux。
- SQLite 迁移 `PRAGMA foreign_keys=OFF`；PATCH 用 COALESCE 逐字段更新。
- 磁盘上的 agent 配置视为代码的一部分：改 hooks/prompt 结构必须让 `refresh_hooks`/`refresh_agent` 在下次启动自愈。
- `#[cfg]` 平台 gate 绑定的是下一个 item：在 gate 下插入函数前确认 gate 仍指向原目标；桌面侧用源码契约测试守住 Android 编译面。

**禁止**
- 重写底层工具已有的能力（进程管理、会话恢复、工具循环、消息总线）。
- 进程内总线、必须长连的守护进程、只有服务器能解读的私有协议。
- 用窗口名、pane 文本或进程存在与否作为"身份"或"真相"。
- 在两个地方写同一个后端事实（resume 方言、白名单、hook 事件名）。
- 在通用模块（`tmux.rs`）里按 backend 字串分支；按屏幕内容触发的适配要有测量与测试。

## 3 · Review 清单

- [ ] 这个改动属于连接/房间/身份/窗口哪一样？若都不是，原生入口找过了吗？
- [ ] 新增的状态存在哪里？能从 state.db + `.tmm/` 重建吗？重启后还对吗？
- [ ] 服务器进程此刻被 kill，人或 agent 会卡住吗？
- [ ] 是否出现了第二份判定逻辑（managed、detect、valid、backend 列表）？
- [ ] 后端字面量（`"kiro"|"claude"|"codex"|"grok"|"omp"`）是否出现在后端文件之外？
- [ ] tmux 命令的目标与格式串查过对应版本文档吗？
- [ ] 有没有同步 I/O 落在 tokio worker 上？有没有锁跨 await？
- [ ] 迁移能在旧库上跑吗？`foreign_keys=OFF` 了吗？
- [ ] 改了 hooks/prompt/配置结构，已 spawn 的 agent 下次启动会自愈吗？

## 4 · 教训记录（证据）

- 2026-08-18 无 recipe 的重启用用户空间配置启动 → agent "能答但聋" → `launch.json`。
- 2026-09-03 `spawn` 曾最后写 recipe 且忽略错误 → 满盘时活着但重启即聋 → 先写后起。
- 2026-09-08 重启原样重放 recipe → 新 AGENTS.md 永远到不了 → `refresh_agent` 重新物化。
- 2026-09-03 `kill_session("dev")` 杀掉 `dev-2`（`-t` 前缀匹配）→ `=name:`。
- 2026-06-15 tmux ≥3.4 八进制转义 `\x1f`，分隔符失效（前一天刚从 `|` 换过来）。
- 2026-09-03 `@all` 扇出内联在 tokio worker 上，卡死所有连接的 push；capture tick 持 store 锁遍历 tmux。
- 2026-08-30 `#[cfg]` 下插函数偷走 gate，两个 commit 后 Android 10 个编译错。
- 2026-09-07 后端白名单两处，omp 能 spawn 但 `registry save` 拒绝 → `SPAWNABLE_BACKENDS`。
- 2026-09-03 七处手拼检测 haystack 一处漏窗口名 → `detect_pane`。
- 2026-09-03 `agent_remove("../..")` 可删工作区 → `valid_name` 白名单在入口。
- snapshots 表：实测每项目 1 条、`restore` 不改投影 → 删除。
- agora 总线：agent 靠 `wait` 长连，服务器死则全队失声 → 所有者决定整个删除。
- `auto_adopt_once`：每个会话都是项目，服务器跟着 tmux 走。

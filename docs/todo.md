# TODO · 现状与信条的差距

> 草案 · 2026-09-09。本文只记录差距与待办，不做决定；决定在 `tenet.md`，规则在
> `guidance/`。历史悬案的详细上下文仍在 `docs/unresolved.md`，这里只给一行与优先级；
> 两者合并是下面的第一条待办。
> 优先级：**P0** 信条已定但代码违反 · **P1** 影响正确性/安全 · **P2** 质量债 · **P3** 记录在案。

---

## A · 信条落地（P0）

- [x] **删除桌面 Team / agora 总线，不留冗余**（所有者 2026-09-09；board #100/#107，2026-09-09 完成）。
  前置搬移落在 `projects/backends/shared.rs` 与 `projects/skills.rs`；Hub 消息底座迁入
  state.db `hub_msgs`（`projects/rooms.rs`，含 proj:* 历史一次性导入）；agora crate、
  `src-tauri/src/team/`、`team_bridge.rs`、`server/team_rpc.rs`、`src/lib/team/`、`team/`
  目录与 `TEAM_*` 配置整体删除；`team.md` 等归档进 `exec-plans/`。
- [ ] **后端知识集中**：一个 `Backend` trait/enum，每个后端一个文件，集中检测 needle、启动命令、
  resume 方言、render、装 hooks、读 hooks 载荷、effort 枚举、状态行 sniff、图标与颜色名。
  `materialize`/`refresh_hooks`/`normalize`/`resume_command` 退化为 trait 调用；前端从服务器取
  后端列表与资源名。守护：source-contract 测试禁止 `backends/` 之外出现后端字面量（测试与
  seed 数据例外）。现状盘点见 §D。
- [ ] **`tmm-cli.md` 瘦身**（所有者："太长了不对"）：2335 行。目标：命令参考一屏，规则回到各
  design doc，历史进 exec-plans。同时审视每个命令是否回答"agent 没有它就做不到什么"。
- [ ] **英文化**：`tenet.md`、`guidance/*.md`、本文定稿后翻成英文（所有者决定文档语言为英语）；
  是否保留 `.zh.md` 待定。
- [ ] **接入地图**：CLAUDE.md 的非负责项与文档地图指向 `tenet.md` 与 `guidance/`；
  `<config>/AGENTS.md` 引用 Zen 短句版（只说流程，见信条十一）。
- [ ] **合并 `docs/unresolved.md` 进本文**或反向，只留一个待办入口。
- [ ] **review 流程**：定义"几个 agent 各审一个维度"的具体做法（每维度一个 reviewer、
  用对应 guidance 的清单、结论写到 board note）。

## B · 正确性（P1）

- [ ] registry def 修改不达已 spawn 的 agent：需 `slots.agent_def` 列，`refresh_hooks` 再同步
  model/mcp/resources、不碰 prompt（`refresh_agent` 已部分解决，确认剩余）。
- [ ] `is_managed_in` 在 `agent_remove` 后可被 kiro 重建 `KIRO_HOME` 子树重新武装 → 门控 `launch.json`。
- [ ] `Terminal.svelte` 触摸手势状态机 ~1300 行嵌在 effect 闭包，零测试；`kbLocked` 不变量只靠文档。
- [x] #108：`onData` 分片/合并响应已由有界状态过滤器覆盖；原始 `?62;22;52c` 现象的来源仍待实测，见 `unresolved.md`。
- [x] #109：完整快照会恢复历史；已修复同步 `clear()` 的假到尾事件、标记丢失与重复重绘，清除改为帧内 `CSI 3J`。
- [ ] 遥测按窗口 INDEX 而身份是 NAME（`renumber-windows` 下错位）；hook→consume 间重命名丢 post；
  相同 body 混淆回执；投递无背压；`SPAWN_CAP` 计入非我方窗口。
- [ ] backend parity：claude `/` palette 未转录；claude/codex/grok 无 auto-continue；codex 无 StopFailure。
- [ ] CSP 真机冒烟（markdown、PDF、mermaid、Hub）；browser/PWA 无 CSP 头。
- [ ] Android 签名 key 曾在 git 历史（60992d4 前），未轮换——所有者决定，记录在案。
- [ ] `adopt_then_down_then_up_restores_the_workspace` flaky → 独立 tmux socket。
- [ ] 三份 shell quoter（`agent_notifications.rs`、`tasks.rs`、`team/backends.rs`）→ 一份。
- [ ] `auto_adopt_with` 在 store 锁下调 tmux。
- [ ] `@all` 收件人存为 `'all'` 但 `pickLead` 不恢复；`hubLog` 有 `before_seq` 时丢 `since_ts`。
- [x] vitals 与 pane 刮取的态度——**已决定**（所有者 2026-09-09）：读人也看得见的屏幕是合规
  观察；`statusLine` 只是显示设置，不改变 agent 行为，允许。剩余工作只是把 sniff 收进后端文件（§A）。

## C · 质量债（P2）

- [ ] `Hub.svelte` 4094 行：Feed / Composer / Roster / Sidebar / Drawer / Dialogs 是连续块，
  耦合状态 `selected`/`agents`/`feed`/`following`/`recipient`/`roomCache`；`onGoBack` 链需 layer-stack。
- [ ] `store.rs` 2989 / `projects/mod.rs` 2223 / `spawn.rs` 2364 / `bin/tmm.rs` 1312 / `vitals.rs` 1308：
  store → projects/registry/board/activity；mod.rs skills 块 → `skills.rs`；spawn `render_*` → 后端文件。
- [ ] `hub_rpc.rs` 750 行 match 混 dispatch/delivery/board 通知策略；~40 处 `require_str→match→err`
  样板 → `?`-returning 内层函数。
- [ ] `Store::hub_search`（board #107）在 Rust 里全表扫描、进程内过滤——当前规模没问题，
  房间变大前把匹配下推到 SQL（`lower(body) LIKE`）或 FTS（reviewer 2026-09-09）。
- [x] #110：Files 的导航历史/返回决策进入 `file-nav.ts`；预览 body/CSS 与 renderer 进入 `FilePreview.svelte` / `file-preview.ts`，保持原行为。
- [ ] Follow up #110: move renderer state down into `FilePreview` so the host
  passes only the file and callbacks, instead of binding `showAllLines` and four DOM references.
- [ ] markdown CSS 重复 → `ui/MarkdownBody`（#110 仅搬移原样式，不跨页面统一）。
- [ ] `ws.ts` 模块级单例（10 个顶层 `let`），split-screen 双连接时是墙。
- [ ] 测试缺口：`bin/tmm.rs` 3 个测试全是 flag 解析；`connection.rs`/`fs.rs`/`server/mod.rs` 无测试；
  `AgentsPage`/`Projects`/`Settings`/`GitPanel`/`ui/Select` 无测试；部分 source test 钉实现文本。
- [ ] `list_panes` 每次全量 `ps -axo`，一次 `hub_post` 触发 ≥2 次；`child_cmd` 是检测线索，先测量。
- [ ] clippy 结构性：`handle_connection`(9)/`handle_connection_ws`(11)/`prepare_codex`(9) 参数过多 →
  上下文结构体；`Outbound::InitCipher` large_enum_variant。
- [ ] 前端后端列表：`AgentsPage.svelte` 与 `TeamTemplates.svelte` 各一份 `BACKENDS`，五处 `?? 'kiro'`
  隐含默认 → 由服务器 `SPAWNABLE_BACKENDS` 提供（并入 §A 后端集中）。
- [ ] `fs_*`/`/dl` 任意绝对路径、git allowlist 含 push/commit：设计如此（token = shell access），
  但 allowlist 读起来像限制——改文档或改名。
- [ ] npm 别名为 pnpm，`package-lock.json` 落后；preflight 检测 `npm_config_user_agent`。
- [ ] Files markdown 预览转义内联 HTML（README badge 变文本）——一个安全渲染器的代价，接受或做白名单。

## D · 五后端现状盘点（供 §A 第二条使用，2026-09-09 按代码读出）

`backend` 是 `&str`，五个字面量散在：

| 文件 | 提及后端名的行数 | 在做什么 |
|---|---|---|
| `projects/spawn.rs` | 218 | 5 个 `render_*`、5 个 `*_hooks`、resume 方言、refresh 探测 |
| `agent_notifications.rs` | 92 | hook 载荷归一化（`normalize` + `is_user_prompt_submit`） |
| `projects/agents.rs` | 89 | 检测表 `KNOWN`（含 resume 字串）、`SPAWNABLE_BACKENDS` |
| `projects/vitals.rs` | 83 | 4 个 `sniff_*` |
| `projects/store.rs` | 80 | seed 定义、排序 `CASE WHEN`、默认模型 |
| `projects/models.rs` | 36 | effort 枚举、模型列表 |
| `team/backends.rs` | 37 | 被 `projects/` 借用的 MCP/启动脚本/信任标记助手 |
| 前端 `hub.ts`/`core/agents.ts`/`AgentsPage`/`TeamTemplates` | 24+24+11+2 | 图标、颜色、命令面板、两份列表、五处默认 |

**不是问题的部分**（所有者）：五个 CLI 的配置格式、hooks 方言、状态行确实不同，各一份实现
是必要的。问题在散落：

1. 没有 `Backend` 类型；同一后端的知识散在 ≥12 个 match/if。2026-09-07 加 omp 漏了 `registry_save`。
2. 同一事实两份表：resume 方言在 `agents.rs::KNOWN` 与 `spawn.rs::resume_command`；前端正则镜像 `find_word`。
3. 一个后端的 hook 契约拆在两个文件（装：spawn.rs；读：agent_notifications.rs）。
4. `refresh_hooks` 按后端专属文件路径探测，而 `launch.json` 记着 backend；加 kiro 回填特例。
5. 五个 `render_*` 复制同一骨架（load 通知中心 → helper → mcp_defs → prompt 文件 → hooks →
   model/effort → `Rendered`）；effort 三种投递散在各函数。
6. `projects/` 依赖将删的 `team/`（见 §A 第一条）。
7. pane 刮取：合规观察（见 §B），但逐后端的 `sniff_*` 应随其他后端知识一起收进后端文件。
8. 前端自己维护后端列表、图标、颜色、命令面板（见 §C）。
9. 通用 tmux 层含 codex 200ms 与 kiro 文件选择器判定——靠屏幕内容触发、有测量，可接受，
   但要承认是后端知识落在通用模块。

## E · Board 上未收口（2026-09-09，全部在 review 列）

#73 CLAUDE.md 瘦身与 docs 分区 · #74 Agent Team 整组拉起 · #75 丢弃 `idle_prompt` 假 waiting ·
#76 terminal 按钮优先当前收件人窗口 · #77 侧栏关闭/移除改菜单 + 二次确认 · #78 长消息已读
确认 · #79 dispatch 全量投递 · #80–#84 安全 review 修复（CSP 待真机）· #88 页头路径可选/
双击复制 · #89 卡片菜单加重启 · #90 To all · #91 选卡片联动 terminal 抽屉 · #92 侧栏只显
agent 窗口 · #94 桌面 Agents 三栏 · #95 Board 滑块圆角 · #97 中文字形 · #98 可点击确认样式 ·
#99 路径链接在 Files 抽屉打开。

## F · 记录在案（P3）

emoji 宽度 tmux 2 格 vs xterm 1 格 · 书签/recents 跨客户端 last-writer-wins · iOS target ·
xterm 换字号后 helper textarea 监听器 · `newWindow` 依赖 `listPanes` 顺序 · 窗口切换器可能显示
非活跃 pane · `slow_rpc_does_not_block_fast_rpc` 只能单向证明并发 · Team 相关悬案（随 Team 删除消失）。

# Guidance · Agent 桥 · The agent bridge

> 信条：三（真实的 CLI）、五（三个原语）、六（CLI 优先）、八（推导不自报）。
> 审这个维度的人问的是：**这条消息是怎么进 pane 的？这个状态是从哪条边推出来的？
> agent 自己能不能做到这件事？人在 terminal 里看得见吗？**
> 草案 · 2026-09-09。

## 1 · 原则

1. **三个原语，没有第四个**：打进 pane 的文本（进）、hooks（被动出）、`tmm`（主动出）。
   任何协作能力都必须能还原为其中之一。
2. **不碰 harness**：所有注入走 CLI 文档化的入口。prompt 是文件，模型与 effort 在配置里，
   MCP 走原生配置，hooks 装在它自己的 hooks 位置。harness 指行为；只影响显示的设置
   （Claude `statusLine`）不改变 agent 做什么，允许。读屏幕（vitals、恢复检测）是观察，允许。
3. **状态从 turn edge 推导**：`userPromptSubmit` 开、`Stop`/`StopFailure` 关。pane 活动不
   是工作，`idle_prompt` 不是提问，agent 的 `tmm status` 是自述不是状态。
4. **一条回复边**：hooks 捕获最终回复，回给开这一回合的那一方；`[reply]` 不造反向边；
   hook 来源的文本只记录不投递。
5. **人读的和 agent 读的是同一份**：房间是唯一记录；打进 pane 的每一行人都读得懂。
6. **prompt 说流程，不上价值**：教会 agent 消息如何流动、如何查历史、如何 `@` 回去、
   如何 done；其余留给工具自己的文档与 skills。
7. **用 prompt 约束的行为迟早失败，必须落到机制**。

## 2 · 必须 / 禁止

**必须**
- 投递前判断目标是 agent 的输入而不是 shell（shell 会执行它）。
- 文本与 Enter 之间等 200ms（codex 0.148.0 将连发 Enter 当粘贴）；含 `\n` 的文本走
  bracketed paste（extended-keys 下 tmux 丢原始 `\n`）；kiro `@` 文件选择器开着时先 Escape
  （靠 footer 两半判定，不靠 backend）。
- 投递确认靠 `userPromptSubmit` 回显匹配：空白盲、截断感知、队列而非单槽、跨重启持久。
- 每套能 auto-post 的 hook 配置都装 turn-start hook。
- 自动恢复：同一 error 只发一次，指数退避，靠 hooks 确认生效而不是靠屏幕文字消失。
- 中断：先重置推导状态再发 Escape（外部取消没有 stop 边）。
- hook helper：fire-and-forget、读一行不等 EOF、经 `/bin/sh`、无 `TMUX_PANE` 即丢弃、
  inbox 扫描在 blocking pool。
- 一个后端的 hook 契约（装什么、读什么）放在同一个文件里。
- 发给 agent 的内容：完整（不截断）、顺序像交接（谁分给谁、主体、note、请做什么）、简洁。
- board 状态被人手动改动时通知被 assign 的 agent（标 done 除外）。

**禁止**
- 无 brief 时发合成首条消息；发 agent 看不到的"仅系统可见"消息。
- 后台替 agent 说话；静默注入 prompt 文本。
- `tmm` 阻塞或成为 agent 工作的前提；`tmm` 失败影响 agent 继续工作。
- 用 pane 活动、屏幕关键字、agent 自述推导 running/waiting。
- 从屏幕全文扫词读 effort/模型（锚定到结构：agent 名所在行、列 0 的 paint block）。
- 在 prompt 里重复 `tmm --help` 已有的内容、加价值观、加"以防万一"的规则。

## 3 · Review 清单

- [ ] 新增的消息路径能还原为三个原语之一吗？
- [ ] 打进 pane 的文本人读得懂吗？带 `[tmm chat …]` 戳吗？
- [ ] 目标 pane 确认是 agent 输入而不是 shell 吗？
- [ ] 状态变化对应哪条 hook 边？有没有从屏幕或自述推导？
- [ ] 这个 hook 事件在每个后端的方言里都测过了吗？版本号写了吗？
- [ ] hooks 装的位置与读的逻辑在同一个文件吗？
- [ ] 已 spawn 的 agent 下次启动会拿到新 hooks/prompt 吗？
- [ ] 自动发送的东西（recovery、continue、通知）有去重、有退避、有 hooks 确认吗？
- [ ] 对应的 `tmm` 子命令有了吗？UI 动作与 CLI 对等吗？
- [ ] prompt/AGENTS.md 的改动是否只说流程？字数能不能再减？

## 4 · 教训记录

- 2026-08-16 `window_activity` 新于 stop 即 working → 所有 agent 永远 working。
- 2026-08-16 hook 来源文本被投递 → @name 打进对方 pane → stop → 回帖 → ping-pong；
  `record_only` 在调用点强制。
- 2026-08-16/22 `userPromptSubmit` 只装在全局配置；claude/codex 缺此 hook → 第一次
  `tmm send` 后 sticky 标志杀掉该窗口此后所有 auto-post。
- 2026-08-19→09-03 投递确认链 7 次：排队行误判 unconfirmed；单槽 pending 被覆盖；换行被
  tmux 丢弃；重启丢队列；服务端 echo 截断 1024 字长消息永不确认；kiro 2.18 `@` 选择器
  吃掉 Enter；1041 字粘贴永远空心环。
- 2026-08-21 `tmm done` 抑制 auto-post → 每个以 done 结尾的 turn 丢最终回复 → 逐字相同才跳过。
- 2026-08-26/27 错误文字长留屏幕 → 重复发 continue → `request_id` 签名 + hooks 确认。
- 2026-08-26 effort 全文匹配命中表格单元 → 永久幽灵。
- 2026-08-29 外部 Escape 无 stop 边 → 卡在 running → 先 reset 再 Esc。
- 2026-08-31/09-05 kiro 硬换行使 `-J` 拼不回、错误头滚出 fold → 恢复检测静默失效。
- 2026-09-02 Claude `idle_prompt` 在 completed 后 60s → 假 waiting（board #75）。
- 2026-09-03 board dispatch 400 字截断 → agent 信息不全（#79）；note 未随 assign 发送（#39）。
- 2026-09-08 `tmm status waiting|blocked` 砍掉、通知 UI 退役：机制越少，agent 理解成本越低。
- Codex "always end with wait" 写进 prompt 仍失败 → Stop hook keepalive。
- 2026-08-18 "多此一举"：无 brief 不发合成首条消息。
- 2026-09-07 所有者高优先级指引：上下文不清先 `tmm log --grep`、直接问对方；被 @ 必 @ 回；
  积压合并回复。

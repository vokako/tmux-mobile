# Guidance · 代码质量 · Code quality

> 信条：一（第一性原理）、九（一种机制）、十一（克制）。
> 审这个维度的人问的是：**这是根因还是症状？这件事在仓库里已经有一份实现了吗？
> 能不能更短？异步落地时写的是对的对象吗？**
> 草案 · 2026-09-09。

## 1 · 原则

1. **根因优先**：fix 的 commit 正文先写根因一句话，再写修法。同一区域第二次出问题，
   默认上次修的是症状。先确认故障在哪一层（浏览器扩展、xterm、tmux、CLI、我们）。
2. **一处定义**：一个概念一个函数、一个原子一个位置、一个规则一个 writer。第二份实现
   一定漂移，而且静默。
3. **克制**：删优于加，短优于长。文件变短是加分项；旧机制被取代就整个删，不留兼容层。
4. **身份在手势时刻冻结**：任何跨 `await`/回调落地的写操作，入口捕获目标身份
   （project、session、issue、window），落地时校验，stale 一律 no-op。
5. **"空"是结论不是默认值**：首个回答未到前不渲染空态；一次失败的读取不清空上次已知
   状态；空态需要 `ready` 门控。
6. **散布的布尔标志是 bug 的温床**：`kbLocked`、`sent_this_turn`、`ctrlArmed`、`noteCopied`
   都出过事。一个不可观测的规则要有一个 owner 函数和一个 writer。
7. **估算常数改测量**：`perLine=80` 在 420px 低估 2 倍；布局事务自带重测。

## 2 · 必须 / 禁止

**必须（TypeScript / Svelte）**
- 相对导入显式 `.ts`；erasable syntax only；`npm run check` 是唯一类型检查。
- 平台判断只从 `core/platform.ts` 导入；`isAndroid` 先于 `isTauri`；`await tauriReady`。
- 新模块同 commit 带 `<module>.test.ts`；可抽成纯函数的逻辑抽出来测，source 正则是最后手段。
- 大数据 base64 分块 8192；HTML 预览 iframe 只 `allow-same-origin`。
- `.js → .ts` 逐文件转换，同 commit 不改逻辑。

**必须（Rust）**
- 同步 I/O 进 `spawn_blocking`；`std::Mutex` guard 不跨 `await`；不在锁内做外部观察。
- 一个 `?`-returning 内层函数处理 RPC，而不是 ~40 处 `require_str → match → err` 样板。
- 一个 shell quoter（目前三份）。
- 参数 ≥ 8 的函数改用上下文结构体。

**禁止**
- 新建与 `ui/Select`、`ui/ContextMenu`、`menuPlacement`、`.to-tail`、`.live-dot`、
  `CreateProjectDialog`、`DirPicker`、`core/markdown` 平行的实现。
- 复制一段样式/逻辑到第二个组件；scoped CSS 重声明 `app.css` 的共享类。
- 用 live 的 `selected`/`cur`/`cwd` 解析异步落地的目标。
- `catch { list = [] }` 之类把失败当空。
- 硬编码字号、颜色、时长、列宽、每行字符数。
- 一个文件超过约 2000 行还在往里加（`Hub.svelte` 4094、`store.rs` 2989、`spawn.rs` 2364、
  `Terminal.svelte` 2555 是待拆的债，不是许可）。
- 在同一 commit 里混机械移动与逻辑改动。

## 3 · Review 清单

- [ ] commit 正文写了根因吗？说明了为什么之前的现象是症状吗？
- [ ] 同类问题在别处排查过了吗（"你再检查一下其他类似操作逻辑"）？
- [ ] 这个函数/组件/样式在仓库里已经有一份了吗？搜过了吗？
- [ ] 改动让哪个文件变短了？能不能删而不是加？
- [ ] 每个 `await` 之后写入的目标是入口时捕获的，还是 live 读的？
- [ ] 空列表/空态是"确认为空"还是"还没问到"？失败会清空上次状态吗？
- [ ] 新增了布尔标志吗？它有几个 writer？
- [ ] 有硬编码的数字或颜色吗？
- [ ] 平台检查顺序对吗？plugin 调用等了 `tauriReady` 吗？
- [ ] 类型检查、`npm test`、`test:rust` 都过了吗？负向验证做了吗？

## 4 · 教训记录

- 2026-08-24 右键 Close/Delete 操作"当前选中项目"而非被长按的行；同日全库排查再补 3 个
  poller 洞；2026-08-31 附件 stage 跨房间泄漏连改三轮；2026-09-01 Board `load()` 用 live
  `cur` 落盘、clipboard resolve 后才盖章——同一病：身份未冻结。
- 2026-08-19 `catch { agents = [] }` 一次超时清空 roster；2026-08-25 切项目闪现"添加 agent"
  面板；DirPicker 每点一次先清空再重画。
- 2026-05-04 chip 三处样式漂移 → `AgentChip`；光标数学两处不一致；平台检测五处；
  39 个组件各写 mono 字体栈；DirPicker 两份；to-tail 两种。
- 2026-05-06 cols×rows 由 6 条路径维护 → 一个 `ResizeObserver`。
- 2026-09-03 Ctrl 一次性标志变永久闩；全局 `noteCopied` 跨 issue 误关。
- 2026-08-31 `perLine` 硬编码 80 在 420px 低估 2 倍（board #46）。
- Escape 丢焦点 6 个 commit（含 objc2 原生补丁与 revert）→ 浏览器扩展："避免我们过度修复了"。
- 键盘 resize 一天四次反转 → "overlay 不 resize"；键盘开启方式三次反转 → 双击在 touchend。
- 网络健康度五次迭代 → WS PING/PONG；工具栏 paint 三连败 → "structure beats paint"。
- 2026-09-03 一日 70 个 fix 的 review：`hub_rpc.rs` 750 行 match、40 处样板、三份 quoter、
  `list_panes` 每次全量 `ps`——都是"记录、暂不动"，理由写在 unresolved.md。

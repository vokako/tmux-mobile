# Guidance · 安全 · Security

> 信条：八（输入在入口校验一次）、二（token = shell access 是设计，边界要写清）。
> 审这个维度的人问的是：**这个字符串从哪来、到哪去、中间经过哪些解析器？
> secret 会落到哪里？渲染的 HTML 是谁的？**
> 2026-09-03 一次专项 review 一天修了 11 个安全项——说明日常 review 缺这个维度。
> 草案 · 2026-09-09。

## 1 · 原则

1. **每个接受名字/路径/URL 的入口只有一个 validator**，是它所经过的所有解析器约束的
   交集白名单（agent 名：目录组件 + tmux 目标 + CLI 参数 + `@地址`）。
2. **文本转义不等于属性安全**：`&`/`<` 让文本惰性，但 `href` 里的 `"` 结束属性，`javascript:`
   在 scheme 位置就是可执行。每个上下文有自己的 guard。
3. **一个渲染器**：所有 markdown 走 `core/markdown` 的 `marked` 单例（`markedSafeUrl` 已注册）；
   CSP `script-src 'self'` 是第二道防线，不是第一道。
4. **secret 不进 stdout、不进仓库、不进日志**：token、API key、keystore 口令。
5. **对外能力宽松是设计，就把它写成设计**：`fs_*`/`/dl` 接受任意绝对路径、git allowlist
   含 push/commit，因为"token = shell access"；但 allowlist 不能读起来像它不是的限制。
6. **加密原语按论文用**：每个 cipher 半边一个 owner、单 FIFO 出站、方向不同 key、nonce 不重用；
   token 比较用常量时间。

## 2 · 必须 / 禁止

**必须**
- 名字在进入处 `valid_name`；路径在进入处拒绝逃逸（`..`、绝对路径混入相对根）；文件名先
  `sanitize_filename`。
- markdown：转义 `&`、`<`，绝不转义 `>`；`link`/`image` 只放行 `http`/`https`/`mailto`
  （图片只 `http`/`https`）；scheme 判定先解实体、去控制字符。
- iframe 永不同时 `allow-scripts` + `allow-same-origin`；opener 能力收窄到具体目录。
- git 参数走 `Command::args`，无 shell；只拒 NUL。
- HTTP 请求读到 `\r\n\r\n`；`/dl` 支持 `Range`，重试重新签名（60s 过期）；`BufStream`
  peek-then-dispatch 返回前 flush。
- E2E：方向密钥分离、常量时间比较、`InitCipher` 与数据走同一队列。
- Android 签名：`key.properties` gitignored；构建脚本不含口令。
- hook helper 经 `/bin/sh` 调用（macOS provenance 会以 137 杀直接执行的脚本）。

**禁止**
- 第二个 `marked.parse`、第二个 shell quoter、第二个名字校验。
- 在服务器日志、`tmm` 输出、房间消息里打印 token 或 key。
- `==` 比较 secret。
- 以描述、窗口名、进程名作为"这是我们的 agent"的判据。
- 用黑名单补分隔符（下一个解析器一定漏）。

## 3 · Review 清单

- [ ] 新接受的字符串经过哪些解析器？validator 是它们约束的交集吗？在入口吗？
- [ ] 渲染 HTML 的地方走的是 `core/markdown` 吗？属性上下文有 guard 吗？
- [ ] 有 secret 可能落到 stdout/日志/房间/仓库吗？
- [ ] 新的 iframe/opener/下载能力比现有的更宽吗？为什么？
- [ ] 加密/比较用的是常量时间与正确的 key 方向吗？
- [ ] 删除类操作的目标路径是由 `agents::home_dir` 之类的受控函数构造的吗？
- [ ] 改了 `tauri.conf.json` 的 CSP，真机冒烟测过吗（markdown、PDF、mermaid、Hub）？

## 4 · 教训记录（全部 2026-09-03 专项 review，board #80–#84）

- `[x](javascript:)` 直达 DOM；`https://a.b/x"onclick=` 自动链接成带 `onclick` 的锚 → `markedSafeUrl`。
- Files 自带 `marked.parse`，README `<img onerror>` 在持 token 的 origin 执行 → 一个渲染器。
- `agent_remove("../..")` 解析到工作区本身 → `valid_name` 白名单。
- `keystore.jks` 与口令在 gradle 脚本里 → 出库；key 未轮换（所有者决定，历史仍含）。
- token 打进 supervised 日志；`==` 比较 token；E2E 双向同 key 同 nonce。
- opener 能力 `**` → 收窄；iframe sandbox 收紧。
- CSP 加入但本机无 webview，需真机冒烟；browser/PWA 无 CSP 头（todo）。

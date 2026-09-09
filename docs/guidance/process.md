# Guidance · 流程、测试与文档 · Process, testing, documentation

> 信条：十二（规则挨着设计，带着理由和事故）、十三（隔离、验证、小步提交）、
> 一（先问本质）。
> 审这个维度的人问的是：**这个改动是怎么被验证的？在哪台设备、哪个构建上？文档跟上了吗？
> 测试钉的是接线还是辅助函数？commit 是一个想法吗？**
> 草案 · 2026-09-09。

## 1 · 原则

1. **worktree 隔离**：每个任务一个 worktree，launch checkout 只做读、协调、集成。
2. **一个 commit 一个想法**：验证过就提交，追加不 amend，机械移动与逻辑改动分开。
3. **测试钉接线，且负向验证**：辅助函数绿不代表组件接对了；改坏一处恰好红一个。
   每个回归修复从失败的测试开始。
4. **文档与代码同 commit**：改行为就改对应 design doc 的 Rules and their reasons；规则格式是
   决定 + 原因 + 日期 + 事故/测量。
5. **测量，不猜测**：外部系统的行为在真实版本上测，把版本号写进注释与文档。
6. **"绿了"不等于验证了**：所有者看到的是他正用的客户端——旧 APK、旧桌面包、未硬刷新
   发生过四次。修复要落到真实入口。
7. **大改先讨论**：涉及架构、设计语言、删功能，先在房间给方案。
8. **沟通**：先结论、少字、不重复；被 @ 必 @ 回；积压合并回复；上下文不清先查历史再问人；
   面向所有者中文，文档/代码/commit/注入流程英文。
9. **分工**：lead 拆任务避免冲突、不代劳；对抗评审；不以实现者自测代替独立验收；
   board 流转 todo → doing → review → done，只有 reviewer 移到 done。

## 2 · 必须 / 禁止

**必须**
- 第一次编辑 tracked 文件前建 worktree（`~/work/worktrees/<repo>/<agent>-<task>`，分支
  `agent/<agent>/<task>`），所有命令用绝对路径。
- commit 正文：根因 → 修法 → 验证方式（含设备/版本）→ 关联 issue；co-author trailer。
- 新模块同 commit 带 `<module>.test.ts`；source-contract 测试解释 WHY；能抽纯函数就不写正则。
- i18n 两个语言分支同 commit；README/design doc/脚本随代码更新。
- Rust 测试 `--test-threads=1`；共享 tmux 的 flaky 测试用独立 socket。
- 涉及 UI 的改动附两端截图或实机描述；涉及 CLI 的改动写明测试的 CLI 版本。
- `tmm status working` 随进度更新；完成 `tmm done`；board `move review` 交接。

**禁止**
- 在 launch checkout 改 tracked 文件；吸收别人的脏树；提交 `agent-team-page/`。
- amend 已交接的 commit；一个 commit 里两个 issue 的 hunks。
- 只改代码不改 doc；只改 doc 不改测试；测试钉实现文本而非不变量。
- 自测通过即标 done；跳过 review 直接 done。
- 没有测量就写"应该是"；没有版本号的外部行为断言。
- 提示词、文档、文案里的冗余：写完再删三分之一。

## 3 · Review 清单

- [ ] 改动在 worktree 里做的吗？commit 只含这个任务的 hunks 吗？
- [ ] commit 正文有根因、验证方式、设备/版本吗？
- [ ] 失败测试先于修复存在吗？负向验证做了吗？
- [ ] 测试钉的是组件接线/用户所见，还是辅助函数？
- [ ] 对应 design doc 的规则更新了吗？格式含日期与事故吗？
- [ ] 两端都验证了吗？在哪个构建上？所有者的客户端能看到吗？
- [ ] 外部系统的断言有版本号吗？
- [ ] `npm test`、`npm run check`、`test:rust` 结果贴了吗？
- [ ] 房间里回应了每个 @ 你的人吗？`tmm done` 了吗？board 移到 review 了吗？
- [ ] 方案级改动事先讨论过了吗？

## 4 · 教训记录

- 2026-09-01 共享 checkout：#43 的 commit 误带 #44 的 Hub hunks，中间两个 commit 样式裸奔；
  并行 WIP 多次让别人的 check 变红；中断的编辑器插入落盘成重复测试。
- 2026-08-31 "helpers being green must not cover a rewired component"；2026-08-20 修一个
  "crying wolf" 的测试；8 月底起 fix 正文普遍含 "negative-controlled"。
- 2026-05-03 签名变更后 `cargo test` 早已编不过；fetch-fonts.sh 产物与 index.html 不一致；
  三份文档承诺的双击手势无实现；CLAUDE.md 规则命名不存在的符号。
- 2026-08-30 board #22 source test 全绿但所有者看到旧 macOS 包；#31 #90 #97 #99 反复排查
  "哪个入口/哪个构建"。
- 开发环境陷阱：服务器注入 `release/` PATH 而 tmm 只编了 debug；gradle symlink 指向另一个
  checkout 的 APK；vite 补丁 dev 不生效；版本戳与迁移块之间的构建让 DB 永久缺表 → `Store::heal`。
- 2026-08-05 `adopt_then_down_then_up` 因共享 tmux 状态 flaky → 需 `-S` socket。
- board 教训：#19 三轮才到根因；#97 kiro 三轮改 lang/栈序，claude 定位字体特性；#56 "点击
  穿透"当遮挡修；#89 把"其他选项菜单"做成可见三点按钮；#38 lead 接受全局编号后被推翻。
- 2026-09-02 CLAUDE.md 117KB → 11KB 地图；"源代码文件夹里不应该有 claude.md，入口太乱了"。
- 所有者三次："先讨论方案，不要直接修改代码"；"保持中文语言风格干练，不啰嗦"；
  "流程应该用英文，保持一致性"。
- 2026-09-08 "大家要分工明确，lead 不要过分代劳，不同人要对抗评审，对立统一"。

# tmux-mobile: 手机远程管理 tmux session

## 项目目标
用 Tauri 2 构建一个跨平台应用（iOS/Android/桌面/Web），通过 WebSocket 远程管理 Mac 上的 tmux session。

## 当前状态
已完成基础验证代码：
- `src/tmux.rs` — tmux CLI 封装层（list sessions/panes、capture、send keys 等），7 个单元测试全通过
- `src/server.rs` — WebSocket server（JSON-RPC 风格 API，端口 9876）
- `src/main.rs` — 入口 + 测试

## 你的任务

### Phase 1: 完善 WebSocket Server
1. 加入认证机制（简单 token 即可，连接时验证）
2. 支持 pane 内容实时推送（subscribe 模式，定时 capture 并 diff 推送变化）
3. 错误处理完善
4. 加 README.md

### Phase 2: 初始化 Tauri 2 项目
1. 在项目中初始化 Tauri 2（支持 desktop + mobile）
2. 前端用简洁的 Web 技术（vanilla JS 或 Svelte，不要 React）
3. 前端页面：
   - Session 列表页（显示所有 session 和状态）
   - Terminal 页面（显示 pane 输出，支持输入命令）
   - 连接设置（输入 server IP:port + token）

### 技术要求
- Rust 后端，WebSocket 通信
- Tauri 2 支持跨平台
- 前端轻量，不要重框架
- 代码整洁，有注释

### 注意
- 现有的 tmux.rs 和 server.rs 是验证代码，可以重构但核心逻辑要保留
- 先确保 `cargo test` 全部通过再做其他改动
- 一步步来，先完善 server，再做 Tauri 集成

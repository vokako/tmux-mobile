# tmux-mobile Quick Start

## 1️⃣ Install Dependencies

```bash
npm install
```

## 2️⃣ Run Tests

```bash
cd src-tauri
cargo test -- --test-threads=1
```

Should see: **7 passed**

## 3️⃣ Run Desktop App

```bash
npm run tauri:dev
```

This will:
- Build frontend (Vite)
- Build Rust backend
- Start WebSocket server on ws://0.0.0.0:9899
- Open desktop window

Look for the console output:
```
🔑 Token: <some-uuid>
```

Use this token in the Settings page.

## 4️⃣ Run Web Version (Development)

Terminal 1 (server):
```bash
cd src-tauri
TOKEN=mytoken cargo run --bin server
```

Terminal 2 (frontend):
```bash
npm run dev
```

Open http://localhost:5173 in browser.

## 5️⃣ Build for Production

Desktop:
```bash
npm run tauri:build
```

Output: `src-tauri/target/release/bundle/macos/tmux-mobile.app`

Android:
```bash
npm run tauri:android:build
```

## 6️⃣ Test in Browser

Open `test.html` in browser (needs a local HTTP server):
```bash
npx serve . -l 8888
# open http://localhost:8888/test.html
```

## Troubleshooting

**Port already in use:**
```bash
lsof -ti:9899 | xargs kill
```

**iOS build fails:**
Need to install xcodegen first (see README.md)

**Android build fails:**
Ensure Android SDK + NDK are installed

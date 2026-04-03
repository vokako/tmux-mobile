# tmux-mobile Quick Start

## 1. Install Dependencies

```bash
npm install
```

## 2. Run Tests

```bash
tmux new-session -d -s test   # Ensure tmux is running
cd src-tauri && cargo test -- --test-threads=1
```

## 3. Run Desktop App

```bash
npm run tauri:dev
```

This will start the WebSocket server and open the desktop window. Check the console for the auth token:
```
Token: <some-uuid>
```

## 4. Run Web Version (Development)

Terminal 1 (server):
```bash
cd src-tauri
TOKEN=mytoken cargo run --bin server
```

Terminal 2 (frontend):
```bash
npm run dev
```

Open http://localhost:5173 in browser. Enter the server address (`ws://localhost:9899`) and token.

## 5. Connect from Phone

Run the server on your machine:
```bash
cd src-tauri && TOKEN=mytoken cargo run --bin server
```

On your phone's browser, navigate to `http://<your-ip>:5173`. Enter `ws://<your-ip>:9899` as the server address.

For HTTPS contexts, the app auto-detects and uses `wss://` with E2E encryption.

## 6. Build for Production

Desktop (macOS):
```bash
npm run build:mac
```

Android APK:
```bash
npm run build:android
```

## Troubleshooting

**Port already in use:**
```bash
lsof -ti:9899 | xargs kill
```

**Android build fails:**
Ensure Android SDK + NDK are installed. NDK 28+ required.

**Keyboard issues on mobile:**
Enable Debug mode (gear icon → Debug → On) to see the debug overlay with keyboard height info.

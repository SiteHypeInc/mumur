# Murmur

**Free, open-source dictation app for Mac (Apple Silicon and Intel) and Windows.** Hold a key, talk, let go. Clean, punctuated text appears wherever your cursor is: Slack, Mail, Cursor, Notion, Claude, a terminal, anything.

Offline voice typing with whisper.cpp on your own computer (no account, no internet), or bring your own cloud keys. An open-source alternative to Wispr Flow and Superwhisper, MIT-licensed.

> Murmur by [SiteHype](https://sitehypedesigns.com/?utm_source=murmur). Free forever: your keys or none. If it saves you time, tell a friend.

---

## What you get

**1. It's free, and it runs on your computer.**
Speech turns into text on your own machine with whisper.cpp (Metal on Apple Silicon). No account. After the one-time model download, no internet. With no keys entered, your voice never leaves the computer. Pick the model that fits: Tiny is 75 MB, Large v3 Turbo is 547 MB and handles 99 languages. Want a cloud engine instead? Paste your own key. Switch any time.

**2. Your words, not a rewrite.**
"Exact words only" is on by default. Cleanup adds punctuation and capitals and drops the ums, and that's all. Murmur then compares the result word by word with what you said. If the AI added words or dropped too many, Murmur throws that version away and types your transcript instead.

**3. A dictation doesn't get thrown away.**
Every recording is saved to disk before it's sent, and every result goes into history before it's pasted.
- Long dictations go up in pieces of up to 60 seconds. Network errors are retried. If the cloud still fails and you have an on-device model, that piece is done on your computer.
- If a dictation still fails, the menu bar has **Retry last recording** (the last 5 failed recordings are kept).
- **Recent** in the menu bar lists your last 15. **Paste last dictation** (`Ctrl+Alt+V`) types it again.
- If Murmur can't confirm the paste landed, your text stays on the clipboard, so ⌘V / Ctrl+V still works.

| | Murmur |
|---|---|
| Price | **Free.** MIT license. Optional cloud engines bill you directly, on your own keys |
| Works offline | **Yes.** On-device whisper.cpp, Metal-accelerated on Apple Silicon |
| Platforms | macOS (Apple Silicon `aarch64` and Intel `x64`) and Windows (`.msi` / `-setup.exe`) |
| Model sizes | Tiny 75 MB · Base 142 MB · Small 466 MB · Large v3 Turbo 547 MB (99 languages) |
| Bring your own models | Groq, OpenAI, Anthropic, **Ollama**, LM Studio, any OpenAI-compatible server |
| Writes for the app you're in | Optional styles per kind of app: casual for chat, formal for email, verbatim identifiers in code. Off while "Exact words only" is on |
| Learns your words | Fix a misheard name once and Murmur spells it right from then on (macOS) |
| Source code | All of it, right here |

## Features

- **Hold to talk, tap to lock.** Hold the key for push-to-talk. A quick tap keeps recording until you tap again. Esc cancels.
- **Fn / Globe key on Mac,** or Right Option, Right ⌘, F13, or any combo you like.
- **No clipped first words.** The mic stays warm briefly and keeps 300 ms of pre-roll.
- **Exact words by default.** Cleanup only punctuates, capitalizes and drops filler, then Murmur checks the result word by word against your transcript.
- **Styles per kind of app (optional).** Personal chat, work chat, email, code & terminals, docs, everything else. Each gets formal, casual, very casual, excited, or your natural voice. Turn off **Exact words only** to use them.
- **App & site modes.** Give `linear.app` its own instructions ("write it as an issue"), or have Messages press Enter to send.
- **Dictionary.** Words are sent to the recognizer and to the cleanup model, then enforced in code. "charge bee" becomes "ChargeBee" every time.
- **Replacements.** Say "my calendar link" and it types `https://cal.com/you`.
- **Never lose text.** Every dictation goes into local history before it's pasted.
  - **Recent** (menu bar) lists the last 15. Click one to copy it.
  - **Paste last dictation** (`Ctrl+Alt+V`, `F9` on Linux) types it again wherever you are.
- **Your clipboard survives, and never at your dictation's expense.** Murmur checks the clipboard really holds the transcript before pressing ⌘V. It puts your old clipboard back only once it sees the text land in the field (macOS). If it can't confirm the paste, the transcript stays on the clipboard so ⌘V still works.
- **Private.** Keys live in the OS keychain. History is a local SQLite file you can auto-expire (1/7/30/90 days) or wipe with **Clear history**, which also shrinks the file on disk. Password fields are never read.

## Install

### Download

Grab the latest build from **[Releases](../../releases)**: the `.dmg` for macOS (Apple Silicon `aarch64` or Intel `x64`), or the `.msi` / `-setup.exe` for Windows. Every push to `main` also builds installers under **Actions → CI → Artifacts**.

**macOS first launch:**

1. Drag **Murmur** to Applications and open it. Builds aren't notarized yet, so macOS blocks the first launch. On **macOS 15 or later**: click **Done** on the warning, open **System Settings → Privacy & Security**, scroll down, click **Open Anyway** next to Murmur, and confirm with your password. (On macOS 14 or earlier, **right-click → Open** works.) If macOS still refuses, run:
   `xattr -dr com.apple.quarantine /Applications/Murmur.app`
2. Murmur lives in the **menu bar** (the little waveform) and the Dock. Don't want the Dock icon? Turn off **Show Murmur in the Dock** in Settings. To keep it on your Desktop too, ⌥⌘-drag it from Applications to make an alias.
3. On **Get started**, turn Murmur on under **Input Monitoring** and **Accessibility**. The Keyboard check turns green within a couple of seconds; press your key to confirm.
4. Click **Test** next to Microphone and allow the mic when macOS asks.
   - **Fn doesn't respond, or the mic test says "silence"?** macOS ties permissions to the exact copy of the app, so a grant made while Murmur ran from the DMG or Downloads (or before an update) goes stale. Click **Reset permissions & relaunch** and allow again.
5. Under **Get started**, pick **Free & private** and download a model (Large v3 Turbo is recommended on Apple Silicon), or pick **Cloud** and paste your keys.
6. Click into any text box and hold **Fn**.

**Windows first launch:**

1. Run the `.msi` or `-setup.exe`. The installer isn't code-signed yet, so Windows may show "Windows protected your PC". Click **More info**, then **Run anyway**.
2. Open Murmur. Under **Get started**, pick **Free & private** and download a model, or pick **Cloud** and paste your keys.
3. Click into any text box and hold **Ctrl+Win** (the default; change it in Settings).

### Code signing (why permissions stick)

macOS ties Accessibility, Input Monitoring and Microphone grants to an app's code signature. CI signs every macOS build with the same Murmur certificate (`scripts/macos-sign.sh`, using the `MURMUR_SIGNING_P12_BASE64` and `MURMUR_SIGNING_P12_PASSWORD` repository secrets), so **you grant permissions once and updates keep them**. It's a self-signed certificate, so macOS still asks you to confirm the first open (see above). An Apple Developer ID would remove that too. Forks without the secrets get ad-hoc builds that need re-granting after each update.

### Build from source

Needs Rust (stable), CMake, and Node 18+ (only for the Tauri CLI; the UI has no build step).

```bash
git clone https://github.com/SiteHypeInc/mumur && cd mumur/app
npx @tauri-apps/cli@2 dev      # run it
npx @tauri-apps/cli@2 build    # .app/.dmg on macOS, .msi/.exe on Windows
```

## Engines

**Speech to text**

| Engine | Cost | Notes |
|---|---|---|
| **On this computer** (default) | Free | whisper.cpp. Tiny 75 MB · Base 142 MB · Small 466 MB · **Large v3 Turbo 547 MB** (recommended on Apple Silicon) |
| Groq | Free tier, then pay as you go | `whisper-large-v3-turbo` |
| OpenAI | Paid | `whisper-1` |
| Custom | Free | Any OpenAI-compatible server (whisper.cpp `whisper-server`, speaches, LocalAI) |

**Cleanup** (removes filler, fixes punctuation, applies your style)

| Engine | Cost | Notes |
|---|---|---|
| Built-in tidy-up | Free | Always on as a fallback: drops "um"/"uh" and stutters, fixes "i" → "I", capitalizes. Never touches code |
| **Ollama** | Free | Pick Ollama in Settings → Engines. It shows which models you have and downloads the one you chose in one click |
| Anthropic | Paid (cents) | Claude Haiku 5.5 by default |
| Groq / OpenAI / Custom | Varies | Any chat model |

The cleanup prompt (`crates/murmur-core/src/prompt.rs`) is written so the model never answers or acts on what you dictate. "What's the capital of France" gets typed, not answered. If cleanup fails, times out, adds words you didn't say, or drops too many, you get your exact transcript with basic tidy-up.

## How it was built

Murmur started as a reverse-engineering exercise. We pointed [REA](https://github.com/morluto/rea), an MCP server for reverse-engineering apps, at the open-source apps in this category to learn how they really work, then built our own:

- **[OpenWhispr](https://github.com/OpenWhispr/openwhispr)** (an Electron clone of Wispr Flow). REA rebuilt its app graph: 2,776 JS files and **575 IPC channels**. That made the whole feature surface visible: hotkey → mic → streaming STT → LLM cleanup → paste → learn-from-edits.
- **[VoiceInk](https://github.com/Beingpax/VoiceInk)** (Swift, Superwhisper-style "modes") and **[Handy](https://github.com/cjpais/Handy)** (Rust/Tauri), traced file by file.

Some of what that taught us, and what Murmur does about it:

- **Whisper hallucinates on silence** ("Thank you.", "Thanks for watching!"). Murmur gates silence before upload and filters the known phantom phrases.
- **Learning from your corrections matters.** We built a word-level diff learner with phonetic matching, plus guards so "cloud" never turns into "Claude".
- **Clipboard-swap pasting is universal but fragile.** Murmur restores your clipboard only if it still holds our text, re-activates the target app first, and saves history before pasting.
- **The Fn/Globe key needs special handling on macOS.** Murmur uses a native event tap that tells left from right modifiers.

The full teardown, with prompts, timings and file references, is in **[docs/teardown.md](docs/teardown.md)**. The playbook for running the same analysis on closed apps (Wispr Flow, Superwhisper) is in [docs/reverse-engineering-playbook.md](docs/reverse-engineering-playbook.md).

## How it's tested

```bash
cargo test --workspace     # ~100 unit tests, mock HTTP servers, no keys needed
e2e/run_e2e.sh             # Linux: the real app, end to end
scripts/check-targets.sh   # type-check macOS + Windows from Linux
MURMUR_TEST_MODEL=ggml-tiny.en.bin cargo test -p murmur-local --test real_model
```

- **End to end:** starts the real app on a virtual display with a PulseAudio "microphone". It presses the hotkey, plays speech, and checks that the cleaned sentence lands in a text box named "Slack". It also checks that the app was classified as work chat, the clipboard was restored, and the paste-last shortcut types it again.
- **Real model:** CI downloads `ggml-tiny.en`, transcribes JFK's "ask not what your country can do for you", and runs the full offline pipeline.
- **UI:** the settings window is driven in Chromium. What it saves must deserialize in Rust.

CI runs all of this on every push, then builds the macOS and Windows installers.

## Project layout

```
crates/murmur-core    platform-free logic: hotkey state machine, audio, app classifier,
                      styles/modes, cleanup prompt, dictionary, learner, providers, pipeline
crates/murmur-local   on-device whisper.cpp engine + model downloads
crates/murmur-sys     OS integration: mic (cpal), hotkeys (CGEventTap / Windows hook / X11),
                      paste + clipboard restore, frontmost app, Accessibility reads
app/src-tauri         Tauri v2 shell: tray + Recent menu, focus-safe overlay pill, settings, keychain
app/ui                overlay and settings (plain HTML/CSS/JS)
e2e/                  end-to-end and UI checks
docs/                 teardown + reverse-engineering playbook
```

## Roadmap

- Streaming transcription while you speak (lower latency on long dictations)
- Parakeet (NVIDIA) on-device models
- Windows: read the focused field (UI Automation) for learn-from-edits and context
- Linux: Wayland global shortcuts
- Notarized macOS builds

## Contributing

Issues and PRs welcome. Run `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` before sending.

## License

[MIT](LICENSE) © SiteHype Inc. and contributors.

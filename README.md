# Murmur

**Free, open-source voice dictation for Mac and Windows.** Hold a key, talk, let go. Clean, punctuated text appears wherever your cursor is: Slack, Mail, Cursor, Notion, Claude, a terminal, anything.

Runs **100% on your computer for free** (whisper.cpp, no account, no internet), or plugs into the fastest cloud models with your own keys. Your call, and you can change it any time.

> Built by [SiteHype](https://sitehypedesigns.com) and given away under the MIT license. If it saves you time, tell a friend.

---

## Why another dictation app?

Superwhisper, Wispr Flow and Glaido are good. But they're closed, mostly subscriptions, and when one of them fails to drop your text into the box, you go digging through its history. Murmur is what we wanted instead:

| | Murmur |
|---|---|
| Price | **Free.** Optional cloud costs pennies with your own keys |
| Works offline | **Yes.** On-device whisper.cpp, Metal-accelerated on Apple Silicon |
| Pick speed vs. accuracy | **Yes.** Tiny (75 MB, instant) → Large v3 Turbo (best, 99 languages) |
| Bring your own models | Groq, OpenAI, Anthropic, **Ollama**, LM Studio, any OpenAI-compatible server |
| Never lose a dictation | Saved before pasting. **Recent** menu in the tray, **paste-last** shortcut, full searchable history |
| Writes for the app you're in | Casual in Slack, formal in email, verbatim identifiers in code |
| Learns your words | Fix a misheard name once and Murmur spells it right from then on (macOS) |
| Source code | All of it, right here |

## Features

- **Hold to talk, tap to lock.** Hold the key for push-to-talk. A quick tap keeps recording until you tap again. Esc cancels.
- **Fn / Globe key on Mac,** or Right Option, Right ⌘, F13, or any combo you like.
- **No clipped first words.** The mic stays warm briefly and keeps 300 ms of pre-roll.
- **Styles per kind of app.** Personal chat, work chat, email, code & terminals, docs, everything else. Each gets formal, casual, very casual, excited, or your natural voice.
- **App & site modes.** Give `linear.app` its own instructions ("write it as an issue"), or have Messages press Enter to send.
- **Dictionary that actually sticks.** Words are sent to the recognizer and to the cleanup model, then enforced in code. "charge bee" becomes "ChargeBee" every time.
- **Replacements.** Say "my calendar link" and it types `https://cal.com/you`.
- **Never lose text.** Every dictation goes into local history before it's pasted.
  - **Recent** (menu bar) lists the last 15. Click one to copy it.
  - **Paste last dictation** (`Ctrl+Alt+V`, `F9` on Linux) types it again wherever you are.
- **Your clipboard survives.** Murmur puts your clipboard back after pasting, unless you copied something new in the meantime.
- **Private.** Keys live in the OS keychain. History is a local SQLite file. Password fields are never read.

## Install

### Download

Grab the latest build from **[Releases](../../releases)**: the `.dmg` for macOS (Apple Silicon `aarch64` or Intel `x64`), or the `.msi` / `-setup.exe` for Windows. Every push to `main` also builds installers under **Actions → CI → Artifacts**.

**macOS first launch:**

1. Drag **Murmur** to Applications and open it. Builds aren't notarized yet, so the first time, **right-click → Open**. If macOS still refuses, run:
   `xattr -dr com.apple.quarantine /Applications/Murmur.app`
2. Murmur lives in the **menu bar** (the little waveform), not the Dock. To keep it on your Desktop too, ⌥⌘-drag it from Applications to make an alias.
3. On **Get started**, turn Murmur on under **Input Monitoring** and **Accessibility**. The Keyboard check turns green within a couple of seconds; press your key to confirm.
4. Click **Test** next to Microphone and allow the mic when macOS asks.
   - **Fn doesn't respond, or the mic test says "silence"?** macOS ties permissions to the exact copy of the app, so a grant made while Murmur ran from the DMG or Downloads (or before an update) goes stale. Click **Reset permissions & relaunch** and allow again.
5. Under **Get started**, pick **Free & private** and download a model (Large v3 Turbo is recommended on Apple Silicon), or pick **Cloud** and paste your keys.
6. Click into any text box and hold **Fn**.

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
| **On this computer** (default) | Free | whisper.cpp. Tiny 75 MB · Base 142 MB · Small 466 MB · **Large v3 Turbo 547 MB** (best) |
| Groq | Free tier, then very cheap | `whisper-large-v3-turbo`, ~300 ms. Fastest cloud option |
| OpenAI | Paid | `whisper-1` |
| Custom | Free | Any OpenAI-compatible server (whisper.cpp `whisper-server`, speaches, LocalAI) |

**Cleanup** (removes filler, fixes punctuation, applies your style)

| Engine | Cost | Notes |
|---|---|---|
| Built-in tidy-up | Free | Always on as a fallback: drops "um"/"uh" and stutters, fixes "i" → "I", capitalizes. Never touches code |
| **Ollama** | Free | `ollama pull llama3.2`, pick Ollama in Settings → Engines |
| Anthropic | Paid (cents) | Claude Haiku 5.5 by default: the best cleanup for the money |
| Groq / OpenAI / Custom | Varies | Any chat model |

The cleanup prompt (`crates/murmur-core/src/prompt.rs`) is written so the model never answers or acts on what you dictate. "What's the capital of France" gets typed, not answered. If cleanup fails, times out, or misbehaves, you get the raw transcript.

## How it was built

Murmur started as a reverse-engineering exercise. We pointed [REA](https://github.com/morluto/rea), an MCP server for reverse-engineering apps, at the open-source apps in this category to learn how they really work, then designed something better:

- **[OpenWhispr](https://github.com/OpenWhispr/openwhispr)** (an Electron clone of Wispr Flow). REA rebuilt its app graph: 2,776 JS files and **575 IPC channels**. That made the whole feature surface visible: hotkey → mic → streaming STT → LLM cleanup → paste → learn-from-edits.
- **[VoiceInk](https://github.com/Beingpax/VoiceInk)** (Swift, Superwhisper-style "modes") and **[Handy](https://github.com/cjpais/Handy)** (Rust/Tauri), traced file by file.

Some of what that taught us, and what Murmur does about it:

- **Whisper hallucinates on silence** ("Thank you.", "Thanks for watching!"). Murmur gates silence before upload and filters the known phantom phrases.
- **The best apps learn from your corrections.** We built a word-level diff learner with phonetic matching, plus guards so "cloud" never turns into "Claude".
- **Clipboard-swap pasting is universal but fragile.** Murmur restores your clipboard only if it still holds our text, re-activates the target app first, and saves history before pasting.
- **The Fn/Globe key needs special handling on macOS.** Murmur uses a native event tap that tells left from right modifiers.

The full teardown, with prompts, timings and file references, is in **[docs/teardown.md](docs/teardown.md)**. The playbook for running the same analysis on closed apps (Wispr Flow, Superwhisper) is in [docs/reverse-engineering-playbook.md](docs/reverse-engineering-playbook.md).

## How it's tested

```bash
cargo test --workspace     # ~80 unit tests, mock HTTP servers, no keys needed
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

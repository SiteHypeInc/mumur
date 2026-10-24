# Murmur

Hold a key, talk, let go. Murmur types clean, punctuated text into whatever app has your cursor: Slack, Mail, Cursor, Notion, a terminal, anything.

It does what Wispr Flow and Superwhisper do, built from a teardown of how this category of app works (see [`docs/teardown.md`](docs/teardown.md)):

- **Fast path everywhere.** The mic stays warm for 30 s after a dictation and keeps a 300 ms pre-roll, so the first word is never clipped. Silence is trimmed before upload. Short dictations (under 4 words) skip the LLM.
- **Writes for the app you're in.** Murmur detects personal chat, work chat, email, code, docs, or other, and applies the style you picked: formal, casual, very casual, excited, or your own tone. Per-app and per-site **modes** add custom instructions or auto-press Enter.
- **Your words, spelled right.** Dictionary terms are sent to the recognizer and to the cleanup model. They're also enforced in code, so "charge bee" becomes "ChargeBee" even if the LLM ignores the list. On macOS, Murmur **learns** a new word when you fix a misheard one in the field you dictated into.
- **Never loses a dictation.** If cleanup fails, times out, or answers your words instead of cleaning them, you get the raw transcript. If paste fails, the text is on your clipboard.
- **Doesn't trash your clipboard.** It saves the clipboard, pastes, and restores it, but only if nothing new was copied in the meantime.
- **Bring your own keys, or go fully local.** Groq Whisper does transcription (~300 ms); Claude Haiku 5.5 or any OpenAI-compatible model does cleanup. You can point both at local servers (whisper.cpp `whisper-server`, Ollama, LM Studio) for fully offline use.
- **Private by default.** Keys go in the OS keychain. History stays in local SQLite. Password fields are never read.

## Using it

| | macOS | Windows |
|---|---|---|
| Default key | **Fn / Globe** | **Ctrl + Win** |
| Hold | push-to-talk | push-to-talk |
| Quick tap | locks recording on; tap again to stop | same |
| Esc while recording | cancel | cancel |

Change keys and modes in **Settings → General**. Right Option, Right ⌘, F13 or any combo works.

## Build and run

Requirements: Rust (stable), Node 18+ (for the Tauri CLI only; the UI has no build step).

```bash
cd app
npx @tauri-apps/cli@2 dev      # run from source
npx @tauri-apps/cli@2 build    # .app/.dmg on macOS, .msi/.exe on Windows
```

**First launch on macOS:** the Get started screen walks you through:

1. **Input Monitoring** (for the Fn key). Grant it, then quit and reopen Murmur.
2. **Accessibility** (to paste and read the focused field).
3. **Microphone** (prompted on first dictation).
4. Paste your **Groq** and **Anthropic** keys and press **Test connection**.

Builds are ad-hoc signed (`signingIdentity: "-"`). For distribution, set your Developer ID in `app/src-tauri/tauri.conf.json` and notarize.

CI (`.github/workflows/ci.yml`) runs tests and the end-to-end check on Linux, then builds macOS (Apple Silicon) and Windows installers as downloadable artifacts.

## Fully local mode

```bash
whisper-server -m ggml-large-v3-turbo.bin --port 8080   # from whisper.cpp
ollama serve && ollama pull llama3.2
```

Settings → Engines: set both providers to **Custom**. The defaults point at `127.0.0.1:8080/v1` and `127.0.0.1:11434/v1`.

## Layout

```
crates/murmur-core   platform-free logic (tested on any OS)
  hotkey.rs          hold / tap / toggle state machine
  audio.rs           downmix, resample to 16 kHz, silence gate + trim, WAV
  context.rs         app/site → personal | work | email | code | document | other
  modes.rs           styles per category + per-app/site overrides
  prompt.rs          the cleanup prompt (the product lives here)
  dictionary.rs      phrase replacements + fuzzy/phonetic term snapping
  learn.rs           learn vocabulary from the user's own corrections
  providers/         OpenAI-compatible STT, Anthropic + OpenAI-compatible LLM, key checks
  pipeline.rs        audio → STT → dictionary → LLM → dictionary, with fallbacks
  config.rs          settings (TOML), history.rs (SQLite)
crates/murmur-sys    OS integration
  capture.rs         cpal mic thread, warm stream, pre-roll, level meter
  keys.rs/hotkey.rs  binding parser + matcher; CGEventTap on macOS (Fn, right-side
                     modifiers, swallows combo keys), rdev elsewhere
  paste.rs           clipboard swap + ⌘V/Ctrl+V + guarded restore, auto-send keys
  frontmost.rs       frontmost app, window title, browser URL (macOS), focused text (macOS AX)
  macos.rs           Accessibility / Quartz FFI
app/src-tauri        Tauri v2 shell: tray, non-activating overlay pill, settings, keychain
app/ui               overlay.html, settings.html (vanilla JS, no bundler)
e2e/                 end-to-end and UI checks
docs/                teardown of Wispr Flow-class apps + reverse-engineering playbook
```

## Testing

```bash
cargo test --workspace                 # 67 unit tests (mock HTTP servers, no keys needed)
e2e/run_e2e.sh                         # Linux: real hotkey, fake mic, mock STT/LLM, real paste
node e2e/ui_check.mjs app/ui <defaults.json> <outdir>   # drives Settings in Chromium
scripts/check-targets.sh               # type-check macOS + Windows from Linux
```

The end-to-end test starts the real app on a virtual X display. A PulseAudio null sink acts as the microphone, and a Tk text box named "Slack" is the target. It presses the hotkey with `xdotool` and plays audio while held. Then it asserts four things:

- the text box contains the cleaned sentence;
- the clipboard was restored;
- the app was classified as work chat;
- vocabulary went to transcription.

## Status and known gaps

- macOS and Windows code type-checks in CI but **hasn't been run on real hardware yet**. Treat the first Mac run as a beta.
- Windows: the focused field isn't read yet (UI Automation), so no learn-from-edits or field context there.
- Linux: hotkeys need X11 (rdev). Wayland global shortcuts are not wired up.
- Clipboard restore covers text and images, not rich text or files.
- No bundled local model yet (use Custom → whisper.cpp). Streaming transcription during speech is the next latency win.

# Running REA on Wispr Flow and Superwhisper (on your own Mac)

The cloud session couldn't download either app because the network policy blocks
wisprflow.ai and superwhisper.com. Run these steps on a Mac where the apps are installed.
Only analyze copies you're licensed to use. The aim is to understand the patterns and
then build a clean-room implementation. Don't copy their code or assets.

## 0. Install REA and register it with Claude Code
```bash
npx rea-agents@latest setup        # choose Claude Code, approve, restart the agent
npm i -g rea-agents && rea --help  # optional CLI
```
Native (Swift) analysis needs Hopper, Ghidra or IDA. Setup can install Hopper if you approve it.

## 1. Wispr Flow (an Electron app, so static JS analysis works well)
```bash
APP="/Applications/Wispr Flow.app"
ls "$APP/Contents/Resources"            # look for app.asar (+ app.asar.unpacked)
rea analyze-javascript-application "$APP/Contents/Resources/app.asar" --json > flow.json
# Native helpers (key listener, paste, accessibility) live in app.asar.unpacked or Contents/MacOS
find "$APP" -type f -perm +111 -not -path "*Frameworks*" | head
```
Ask the agent:
> Using REA on flow.json and the Wispr Flow bundle: list every IPC channel, then trace the
> hotkey → audio capture → upload/stream → LLM formatting → paste path. Show the network
> endpoints and the payload shape sent with the audio (app context, dictionary, style).
> How does "Styles" choose per-app tone? How are edits auto-learned into the dictionary?

What to check (each maps to a section in REPORT.md):
- Whether audio streams over a WebSocket during speech or uploads once at release (this decides latency).
- Which context goes up with the audio: app name, URL, AX text before and after the cursor, dictionary.
- Whether the Fn/Globe key is captured with a CGEventTap or an NSEvent monitor, and whether it changes `AppleFnUsageType`.
- Whether text is inserted by pasting through the pasteboard (and how long before the clipboard is restored) or via AX `kAXSelectedText`.
- How edits after paste are watched (`AXObserver` on `kAXValueChangedNotification`).

## 2. Superwhisper (native Swift, so use a decompiler)
```bash
APP="/Applications/superwhisper.app"
rea --help | grep -i -E "apple|bundle|native"     # bundle anatomy and native commands
# Then, through the agent with Hopper or Ghidra configured:
```
> Use REA's Apple bundle inspection on /Applications/superwhisper.app, then native analysis
> on the main executable. Find the "modes" model (per-app mode switching), the whisper.cpp
> or CoreML model loading path, the hotkey implementation, and the text-insertion routine.
> Show the strings and symbols you're relying on.

Quick wins that don't need a decompiler:
```bash
plutil -p "$APP/Contents/Info.plist"
otool -L "$APP/Contents/MacOS/"* | grep -v /System   # bundled frameworks (whisper, sparkle…)
strings -n 8 "$APP/Contents/MacOS/superwhisper" | grep -iE "prompt|mode|whisper|paste|AX" | head -200
ls ~/Library/Application\ Support/superwhisper     # modes are stored as JSON, which shows the schema
```

## 3. Optional: capture network traffic (with your consent, against your own account)
Use mitmproxy or Proxyman to save a HAR, then run `rea` on the saved capture
(`docs/web-network-captures.md` in the REA repo). This shows the exact request and response
contract for cloud transcription and formatting.

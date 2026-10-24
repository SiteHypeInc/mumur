// Drives settings.html in Chromium with a mocked Tauri bridge: exercises every
// section, screenshots them, and writes the last saved settings payload.
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const [, , uiDir, defaultsFile, outDir] = process.argv;
const defaults = JSON.parse(fs.readFileSync(defaultsFile, "utf8"));
fs.mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch({ executablePath: "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 980, height: 720 }, colorScheme: "light" });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));

await page.addInitScript((settings) => {
  window.__saved = [];
  const overview = { settings, keys: { groq: true, openai: false, anthropic: false, custom: false }, accessibility: true,
    input_monitoring: false, hotkey_error: null, devices: ["MacBook Pro Microphone", "AirPods Pro"], platform: "macos",
    version: "0.1.0", total_words: 12840, paused: false };
  const hist = [
    { id: 2, created_at: Date.now() - 60000, app: "Slack", category: "work", raw: "um so can you push it to git hub", text: "Can you push it to GitHub?", audio_ms: 2100, stt_ms: 310, polish_ms: 290 },
    { id: 1, created_at: Date.now() - 3600000, app: "Mail", category: "email", raw: "hi sarah", text: "Hi Sarah,\n\nThanks for the update.", audio_ms: 3000, stt_ms: 280, polish_ms: 350 },
  ];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === "overview") return overview;
      if (cmd === "save_settings") { window.__saved.push(JSON.parse(JSON.stringify(args.settings))); return null; }
      if (cmd === "history") return hist.filter((h) => !args.query || h.text.toLowerCase().includes(args.query.toLowerCase()));
      if (cmd === "test_connection") return "Transcription: Groq ✓ · Cleanup: Anthropic ✓";
      if (cmd === "plugin:autostart|is_enabled") return false;
      return null;
    } },
    event: { listen: async () => () => {} },
  };
}, defaults);

await page.goto("file://" + path.resolve(uiDir, "settings.html") + "#setup");
await page.waitForSelector("#setup.active");
await page.click("#test");
await page.waitForFunction(() => document.querySelector("#test-result").textContent.includes("✓"));
await page.screenshot({ path: `${outDir}/1-setup.png` });

await page.click('nav a[data-s="general"]');
await page.selectOption("#binding-select", "RightOption");
await page.selectOption('select[data-bind="hotkey.mode"]', "toggle");
await page.selectOption("#devices", "AirPods Pro");
await page.screenshot({ path: `${outDir}/2-general.png` });

await page.click('nav a[data-s="engines"]');
await page.selectOption('select[data-bind="stt.provider"]', "open_ai");
await page.selectOption('select[data-bind="stt.provider"]', "groq");
await page.screenshot({ path: `${outDir}/3-engines.png` });

await page.click('nav a[data-s="styles"]');
await page.selectOption('select[data-cat="personal"]', "very_casual");
await page.screenshot({ path: `${outDir}/4-styles.png` });

await page.click('nav a[data-s="modes"]');
await page.click("#add-mode");
const mode = page.locator(".mode").first();
await mode.locator('[data-f="name"]').fill("Linear");
await mode.locator('[data-f="hosts"]').fill("linear.app");
await mode.locator('[data-f="apps"]').fill("Linear, com.linear");
await mode.locator('[data-f="style"]').selectOption("formal");
await mode.locator('[data-f="send_key"]').selectOption("cmd_enter");
await mode.locator('[data-f="instructions"]').fill("Write as an issue: a one-line title, then details.");
await mode.locator('[data-f="polish"]').uncheck();
await mode.locator('[data-f="polish"]').check();
await page.screenshot({ path: `${outDir}/5-modes.png` });

await page.click('nav a[data-s="dictionary"]');
await page.fill("#new-term", "Sinéad");
await page.press("#new-term", "Enter");
await page.fill("#new-term", "ChargeBee");
await page.click("#add-term");
await page.click("#add-rep");
await page.locator('#reps input[data-k="from"]').fill("my calendar link");
await page.locator('#reps input[data-k="to"]').fill("https://cal.com/john");
await page.screenshot({ path: `${outDir}/6-dictionary.png` });

await page.click('nav a[data-s="history"]');
await page.waitForSelector("#hist li p");
await page.screenshot({ path: `${outDir}/7-history.png` });
await page.fill("#hist-q", "github");
await page.waitForFunction(() => document.querySelectorAll("#hist li").length === 1);

await page.emulateMedia({ colorScheme: "dark" });
await page.click('nav a[data-s="modes"]');
await page.screenshot({ path: `${outDir}/8-modes-dark.png` });

await page.waitForTimeout(700); // let the debounced save fire
const saved = await page.evaluate(() => window.__saved);
fs.writeFileSync(`${outDir}/saved.json`, JSON.stringify(saved.at(-1), null, 2));
console.log(`saves=${saved.length} errors=${errors.length}`);
errors.forEach((e) => console.log("ERR", e));
await browser.close();
process.exit(errors.length ? 1 : 0);

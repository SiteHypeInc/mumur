// Renders overlay.html's states with a mocked event bus and screenshots them side by side.
import { chromium } from "playwright";
import path from "node:path";
const [, , uiDir, out] = process.argv;
const browser = await chromium.launch({ executablePath: "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 236, height: 64 }, deviceScaleFactor: 2 });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.addInitScript(() => {
  window.__h = {};
  window.__TAURI__ = {
    core: { invoke: async () => ({ settings: { audio: { sounds: false } } }) },
    event: { listen: async (name, cb) => { window.__h[name] = cb; return () => {}; } },
  };
});
await page.goto("file://" + path.resolve(uiDir, "overlay.html"));
await page.addStyleTag({ content: "html,body{background:#c7d2fe !important}" });
const emit = (n, p) => page.evaluate(([n, p]) => window.__h[n]({ payload: p }), [n, p]);
const shots = [];
await emit("pill", { state: "recording", mode: null });
for (let i = 0; i < 14; i++) await emit("level", [0.1, 0.4, 0.7, 0.5, 0.9, 0.3, 0.6, 0.8, 0.2, 0.5, 0.7, 0.4, 0.6, 0.3][i]);
await page.waitForTimeout(250); shots.push(await page.screenshot());
await emit("pill", { state: "processing" }); await page.waitForTimeout(250); shots.push(await page.screenshot());
await emit("pill", { state: "done", words: 12, ms: 610 }); await page.waitForTimeout(250); shots.push(await page.screenshot());
await emit("pill", { state: "message", text: "Learned: Sinéad", tone: "ok" }); await page.waitForTimeout(250); shots.push(await page.screenshot());
await emit("pill", { state: "message", text: "Groq rejected the API key", tone: "error" }); await page.waitForTimeout(250); shots.push(await page.screenshot());
const sheet = await browser.newPage({ viewport: { width: 236, height: 64 * shots.length }, deviceScaleFactor: 2 });
await sheet.setContent(`<body style="margin:0">${shots.map((b) => `<img style="display:block;width:236px" src="data:image/png;base64,${b.toString("base64")}">`).join("")}</body>`);
await sheet.screenshot({ path: out });
console.log("errors=" + errors.length, errors.join("\n"));
await browser.close();

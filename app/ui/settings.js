const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];

let state = null; // overview from Rust
let s = null; // settings being edited

const CATEGORIES = [
  ["personal", "Personal messages", "iMessage, WhatsApp, Telegram, Discord"],
  ["work", "Work chat", "Slack, Teams, Linear"],
  ["email", "Email", "Mail, Gmail, Outlook, Superhuman"],
  ["code", "Code & terminals", "Cursor, VS Code, Terminal, Claude, ChatGPT"],
  ["document", "Docs & notes", "Notion, Google Docs, Word, Obsidian"],
  ["other", "Everything else", ""],
];
const STYLES = [
  ["natural", "Natural (your own tone)"],
  ["formal", "Formal"],
  ["casual", "Casual"],
  ["very_casual", "very casual"],
  ["excited", "Excited!"],
];
const MAC_KEYS = [["Fn", "Fn / Globe"], ["RightOption", "Right Option"], ["RightCommand", "Right Command"], ["RightControl", "Right Control"], ["F13", "F13"]];
const PC_KEYS = [["Ctrl+Win", "Ctrl + Win"], ["RightCtrl", "Right Ctrl"], ["RightAlt", "Right Alt"], ["F13", "F13"]];

function toast(msg) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.add("on");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => t.classList.remove("on"), 1800);
}

// ---- navigation ----
window.showSection = (id) => {
  if (!document.getElementById(id)) id = "general";
  $$("section").forEach((x) => x.classList.toggle("active", x.id === id));
  $$("#nav a").forEach((a) => a.classList.toggle("active", a.dataset.s === id));
  if (id === "history") loadHistory();
  history.replaceState(null, "", "#" + id);
};
$$("#nav a").forEach((a) => a.addEventListener("click", () => showSection(a.dataset.s)));

// ---- data binding: data-bind="a.b" maps to s.a.b ----
const get = (path) => path.split(".").reduce((o, k) => o?.[k], s);
function set(path, value) {
  const keys = path.split(".");
  const last = keys.pop();
  keys.reduce((o, k) => o[k], s)[last] = value;
}

let saveTimer;
function save(immediate) {
  clearTimeout(saveTimer);
  const run = async () => {
    try {
      await invoke("save_settings", { settings: s });
      toast("Saved");
    } catch (e) {
      toast(String(e));
    }
  };
  if (immediate) run();
  else saveTimer = setTimeout(run, 450);
}

function bindAll() {
  $$("[data-bind]").forEach((el) => {
    const path = el.dataset.bind;
    const v = get(path);
    if (el.type === "checkbox") el.checked = !!v;
    else el.value = v ?? "";
    el.oninput = el.onchange = () => {
      let val = el.type === "checkbox" ? el.checked : el.value;
      if (el.dataset.type === "int") val = parseInt(val || "0", 10);
      set(path, val);
      if (path === "hotkey.binding") syncBinding();
      if (path === "stt.provider") syncEngineViews();
      save(el.tagName === "SELECT" || el.type === "checkbox");
    };
  });
}

function syncBinding() {
  const keys = state.platform === "macos" ? MAC_KEYS : PC_KEYS;
  const sel = $("#binding-select");
  const known = keys.some(([k]) => k === s.hotkey.binding);
  sel.innerHTML = keys.map(([k, l]) => `<option value="${k}">${l}</option>`).join("") + (known ? "" : `<option value="${esc(s.hotkey.binding)}">Custom: ${esc(s.hotkey.binding)}</option>`);
  sel.value = s.hotkey.binding;
  $("#binding-input").value = s.hotkey.binding;
  $("#hotkey-name").textContent = s.hotkey.binding;
  $$(".hk-name").forEach((k) => (k.textContent = s.hotkey.binding));
}

function esc(t) {
  return String(t ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

// ---- setup ----
function renderSetup() {
  renderHotkeyStatus(state.hotkey_status);
  $("#mic-device").textContent = s.audio.device || state.default_input || "";
  $("#hotkey-error").textContent = state.hotkey_error || "";
  $$("[data-key]").forEach((inp) => {
    inp.classList.toggle("saved", !!state.keys[inp.dataset.key]);
    inp.placeholder = state.keys[inp.dataset.key] ? "•••••••• saved" : inp.placeholder;
    inp.onchange = async () => {
      try {
        await invoke("set_api_key", { provider: inp.dataset.key, key: inp.value });
        state.keys[inp.dataset.key] = !!inp.value.trim();
        inp.value = "";
        inp.classList.toggle("saved", state.keys[inp.dataset.key]);
        inp.placeholder = state.keys[inp.dataset.key] ? "•••••••• saved" : "";
        toast(state.keys[inp.dataset.key] ? "Key saved to keychain" : "Key removed");
      } catch (e) {
        toast(String(e));
      }
    };
  });
}
// ---- diagnostics ----
const HK_TEXT = {
  active: ["✓ Ready", "yes", ""],
  listen_only: ["✓ Ready (basic)", "yes", "Works now. Allow Accessibility too so Murmur can paste and keep key combos from typing."],
  waiting_for_permission: ["Waiting for permission", "wait",
    "Turn Murmur on under Input Monitoring and Accessibility. It starts working within a couple of seconds; no restart needed."],
};
function renderHotkeyStatus(st) {
  const [label, cls, hint] = HK_TEXT[st] || HK_TEXT.waiting_for_permission;
  const el = $("#hk-status");
  el.textContent = label;
  el.className = "badge " + cls;
  $("#hk-hint").textContent = state.platform === "macos" ? hint : "";
}
setInterval(async () => {
  if (!$("#setup").classList.contains("active")) return;
  try {
    state.hotkey_status = await invoke("hotkey_status");
    renderHotkeyStatus(state.hotkey_status);
  } catch {}
}, 2000);
listen("hotkey-test", ({ payload: down }) => {
  const live = $("#hk-live");
  live.classList.toggle("hit", down);
  if (!down) {
    live.classList.add("ok");
    live.innerHTML = `✓ Got it, your key works`;
  }
});
$$("[data-pane]").forEach((b) => (b.onclick = () => invoke("open_privacy", { pane: b.dataset.pane })));
$("#relaunch").onclick = () => invoke("relaunch");
$("#reset-perms").onclick = () => {
  if (confirm("Reset Murmur's macOS permissions and relaunch? You'll be asked to allow them again.")) invoke("reset_permissions");
};
$("#mic-test").onclick = async () => {
  const btn = $("#mic-test"), hint = $("#mic-hint"), bar = $("#mic-bar");
  btn.disabled = true;
  hint.className = "hint";
  hint.textContent = "Listening… say something";
  bar.style.width = "0";
  try {
    const r = await invoke("test_microphone");
    bar.style.width = Math.round(Math.min(1, r.level) * 100) + "%";
    $("#mic-device").textContent = r.device || "";
    if (r.max_sample === 0) {
      hint.className = "error";
      hint.textContent = state.platform === "macos"
        ? "Murmur is getting silence: the mic is blocked. Click Open Microphone settings, turn Murmur on, then relaunch."
        : "No audio from this microphone. Pick another one in General.";
    } else if (r.level < 0.05) {
      hint.className = "hint";
      hint.textContent = "Very quiet. Speak up, or pick a different mic in General (Bluetooth earbuds can take a second to switch on).";
    } else {
      hint.className = "hint ok";
      hint.textContent = "✓ Sounds good";
    }
  } catch (e) {
    hint.className = "error";
    hint.textContent = String(e);
  }
  btn.disabled = false;
};
$("#test").onclick = async () => {
  const out = $("#test-result");
  out.textContent = "Testing…";
  out.className = "hint";
  try {
    out.textContent = await invoke("test_connection");
    out.className = "hint ok";
  } catch (e) {
    out.textContent = String(e);
    out.className = "error";
  }
};

// ---- on-device models ----
let models = [];
const progress = {}; // id -> {done,total}
const DEFAULT_MODEL = "large-v3-turbo-q5";
const dots = (n, max = 5) => "<b>" + "●".repeat(n) + "</b>" + "○".repeat(max - n);

async function loadModels() {
  models = (await invoke("models")) || [];
  renderModels();
}

function renderModels() {
  const current = s.stt.model || DEFAULT_MODEL;
  const html = models.length
    ? models.map((m) => {
        const p = progress[m.id];
        const pct = p && p.total ? Math.round((p.done / p.total) * 100) : 0;
        const right = m.downloaded
          ? `<span class="state-ok">✓ Ready</span> <button class="danger" data-delete="${m.id}" title="Delete download">×</button>`
          : p
            ? `<div class="bar"><i style="width:${pct}%"></i></div> <span class="meta">${pct}%</span>`
            : `<button data-download="${m.id}">Download · ${m.size_mb} MB</button>`;
        return `<div class="model ${m.id === current ? "active" : ""}">
          <input type="radio" name="model" data-use="${m.id}" ${m.id === current ? "checked" : ""} ${m.downloaded ? "" : "disabled"} />
          <div><div class="name">${esc(m.label)} ${m.id === DEFAULT_MODEL ? '<span class="opt">recommended</span>' : ""}</div>
            <div class="meta">${esc(m.note)}</div>
            <div class="meters">speed ${dots(m.speed)} &nbsp; accuracy ${dots(m.accuracy)}</div></div>
          <div class="row">${right}</div></div>`;
      }).join("")
    : `<p class="hint">This build has no on-device engine.</p>`;
  $$("[data-models]").forEach((el) => {
    el.innerHTML = html;
    $$("[data-download]", el).forEach((b) => (b.onclick = () => download(b.dataset.download)));
    $$("[data-delete]", el).forEach((b) => (b.onclick = async () => {
      if (!confirm("Delete this model from disk?")) return;
      await invoke("delete_model", { id: b.dataset.delete });
      loadModels();
    }));
    $$("[data-use]", el).forEach((r) => (r.onchange = () => useModel(r.dataset.use)));
  });
}

function useModel(id) {
  s.stt.model = id;
  s.stt.provider = "local";
  bindAll();
  syncEngineViews();
  renderModels();
  save(true);
}

async function download(id) {
  progress[id] = { done: 0, total: null };
  renderModels();
  try {
    await invoke("download_model", { id });
    delete progress[id];
    await loadModels();
    if (!models.some((m) => m.downloaded && m.id === (s.stt.model || DEFAULT_MODEL))) useModel(id);
    else if (s.stt.provider !== "local") useModel(id);
    toast("Model ready. Hold your key and talk!");
  } catch (e) {
    delete progress[id];
    renderModels();
    toast(String(e));
  }
}
listen("model-progress", ({ payload }) => {
  progress[payload.id] = payload;
  renderModels();
});

function syncEngineViews() {
  const local = s.stt.provider === "local";
  $$(".when-local").forEach((x) => (x.style.display = local ? "" : "none"));
  $$(".when-cloud").forEach((x) => (x.style.display = local ? "none" : ""));
  $$(".engine-local").forEach((x) => (x.style.display = local ? "" : "none"));
  $$(".engine-cloud").forEach((x) => (x.style.display = local ? "none" : ""));
  $$('input[name="engine"]').forEach((r) => (r.checked = (r.value === "local") === local));
}
$$('input[name="engine"]').forEach((r) => (r.onchange = () => {
  if (r.value === "local") s.stt.provider = "local";
  else if (s.stt.provider === "local") s.stt.provider = "groq";
  if (r.value === "cloud" && s.polish.provider === "ollama") s.polish.provider = "anthropic";
  bindAll();
  syncEngineViews();
  save(true);
}));

// ---- styles ----
function renderStyles() {
  $("#style-list").innerHTML = CATEGORIES.map(
    ([id, label, ex]) => `<div class="style-row"><div>${label}<small>${ex}</small></div>
      <select data-cat="${id}">${STYLES.map(([v, l]) => `<option value="${v}">${l}</option>`).join("")}</select></div>`
  ).join("");
  $$("[data-cat]").forEach((sel) => {
    sel.value = s.styles[sel.dataset.cat] || "natural";
    sel.onchange = () => {
      s.styles[sel.dataset.cat] = sel.value;
      save(true);
    };
  });
}

// ---- modes ----
const split = (t) => t.split(/[,\n]/).map((x) => x.trim()).filter(Boolean);
function renderModes() {
  const list = $("#mode-list");
  list.innerHTML = "";
  s.modes.forEach((m, i) => {
    const el = document.createElement("div");
    el.className = "card mode";
    el.innerHTML = `<div class="head"><label class="field">Name<input data-f="name" value="${esc(m.name)}" /></label>
      <button class="remove danger">Remove</button></div>
      <div class="grid">
        <label class="field">Apps <span class="opt">names or bundle ids, comma separated</span><input data-f="apps" value="${esc(m.apps.join(", "))}" placeholder="Slack, com.apple.Terminal" /></label>
        <label class="field">Sites <span class="opt">hosts</span><input data-f="hosts" value="${esc(m.hosts.join(", "))}" placeholder="linear.app, github.com" /></label>
        <label class="field">Style<select data-f="style"><option value="">Category default</option>${STYLES.map(([v, l]) => `<option value="${v}">${l}</option>`).join("")}</select></label>
        <label class="field">After pasting<select data-f="send_key"><option value="">Do nothing</option><option value="enter">Press Enter (send)</option><option value="shift_enter">Press Shift+Enter</option><option value="cmd_enter">Press ⌘/Ctrl+Enter</option></select></label>
      </div>
      <label class="field">Instructions <span class="opt">added to the cleanup prompt</span><textarea data-f="instructions" placeholder="e.g. Write as a Linear issue: a one-line title, then details.">${esc(m.instructions)}</textarea></label>
      <label class="check"><input type="checkbox" data-f="polish" ${m.polish === false ? "" : "checked"} /> Use AI cleanup in this mode</label>`;
    $('[data-f="style"]', el).value = m.style || "";
    $('[data-f="send_key"]', el).value = m.send_key || "";
    $$("[data-f]", el).forEach((inp) => {
      inp.oninput = inp.onchange = () => {
        const f = inp.dataset.f;
        if (f === "apps" || f === "hosts") m[f] = split(inp.value);
        else if (f === "polish") m.polish = inp.checked ? null : false;
        else if (f === "style" || f === "send_key") m[f] = inp.value || null;
        else m[f] = inp.value;
        save(inp.tagName === "SELECT" || inp.type === "checkbox");
      };
    });
    $(".remove", el).onclick = () => {
      s.modes.splice(i, 1);
      renderModes();
      save(true);
    };
    list.appendChild(el);
  });
}
$("#add-mode").onclick = () => {
  s.modes.push({ name: "New mode", apps: [], hosts: [], instructions: "", style: null, polish: null, send_key: null });
  renderModes();
  save(true);
};

// ---- dictionary ----
function renderDictionary() {
  $("#terms").innerHTML = s.dictionary.terms.length
    ? s.dictionary.terms.map((t, i) => `<span class="chip">${esc(t)}<button data-i="${i}" title="Remove">×</button></span>`).join("")
    : `<span class="hint">No words yet.</span>`;
  $$("#terms button").forEach((b) => (b.onclick = () => {
    s.dictionary.terms.splice(+b.dataset.i, 1);
    renderDictionary();
    save(true);
  }));
  $("#reps").innerHTML = s.dictionary.replacements
    .map((r, i) => `<tr><td><input data-i="${i}" data-k="from" value="${esc(r.from)}" placeholder="my calendar link" /></td>
      <td><input data-i="${i}" data-k="to" value="${esc(r.to)}" placeholder="https://cal.com/you" /></td>
      <td><button data-del="${i}" title="Remove">×</button></td></tr>`)
    .join("");
  $$("#reps input").forEach((inp) => (inp.oninput = () => {
    s.dictionary.replacements[+inp.dataset.i][inp.dataset.k] = inp.value;
    save();
  }));
  $$("#reps [data-del]").forEach((b) => (b.onclick = () => {
    s.dictionary.replacements.splice(+b.dataset.del, 1);
    renderDictionary();
    save(true);
  }));
}
function addTerm() {
  const v = $("#new-term").value.trim();
  if (!v) return;
  if (!s.dictionary.terms.some((t) => t.toLowerCase() === v.toLowerCase())) s.dictionary.terms.push(v);
  $("#new-term").value = "";
  renderDictionary();
  save(true);
}
$("#add-term").onclick = addTerm;
$("#new-term").onkeydown = (e) => e.key === "Enter" && addTerm();
$("#add-rep").onclick = () => {
  s.dictionary.replacements.push({ from: "", to: "" });
  renderDictionary();
};

// ---- history ----
async function loadHistory() {
  const items = await invoke("history", { query: $("#hist-q").value || null, limit: 200 });
  const words = state?.total_words ?? 0;
  $("#hist-stats").textContent = words ? `${words.toLocaleString()} words dictated · about ${Math.round(words / 40).toLocaleString()} minutes of typing saved` : "";
  $("#hist").innerHTML = items.length
    ? items.map((e) => `<li><div class="meta"><span>${new Date(e.created_at).toLocaleString()}</span><span>${esc(e.app || "")}</span>
        <span>${(e.stt_ms + e.polish_ms)} ms</span><span class="sp"></span>
        <button data-copy="${e.id}">Copy</button><button data-del="${e.id}">Delete</button></div>
        <p>${esc(e.text)}</p>${e.raw && e.raw !== e.text ? `<details><summary>Raw transcript</summary>${esc(e.raw)}</details>` : ""}</li>`).join("")
    : `<li class="hint">Nothing here yet. Hold your key and say something.</li>`;
  $$("[data-copy]").forEach((b) => (b.onclick = async () => {
    const e = items.find((x) => x.id === +b.dataset.copy);
    await invoke("copy_text", { text: e.text });
    toast("Copied");
  }));
  $$("#hist [data-del]").forEach((b) => (b.onclick = async () => {
    await invoke("delete_history", { id: +b.dataset.del });
    loadHistory();
  }));
}
let histTimer;
$("#hist-q").oninput = () => {
  clearTimeout(histTimer);
  histTimer = setTimeout(loadHistory, 200);
};
$("#hist-clear").onclick = async () => {
  if (!confirm("Delete all dictation history?")) return;
  await invoke("delete_history", { id: null });
  loadHistory();
};
listen("dictation", () => $("#history").classList.contains("active") && loadHistory());
listen("learned", async () => {
  state = await invoke("overview");
  s = state.settings;
  renderDictionary();
});

// ---- autostart (plugin) ----
async function initAutostart() {
  const box = $("#autostart");
  try {
    box.checked = await invoke("plugin:autostart|is_enabled");
  } catch {
    box.disabled = true;
  }
  box.onchange = async () => {
    try {
      await invoke(box.checked ? "plugin:autostart|enable" : "plugin:autostart|disable");
    } catch (e) {
      toast(String(e));
    }
  };
}

// ---- boot ----
(async () => {
  state = await invoke("overview");
  s = state.settings;
  document.body.classList.toggle("macos", state.platform === "macos");
  $("#devices").innerHTML = `<option value="">System default</option>` + state.devices.map((d) => `<option>${esc(d)}</option>`).join("");
  $("#foot").textContent = `v${state.version}`;
  syncBinding();
  bindAll();
  renderSetup();
  renderStyles();
  renderModes();
  renderDictionary();
  initAutostart();
  syncEngineViews();
  loadModels();
  showSection(location.hash.slice(1) || "general");
})();

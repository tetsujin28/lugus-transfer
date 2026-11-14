// Lugus Transfer by Domingo Olmo Martin : interface.
// Fonctionne dans l'app Tauri (window.__TAURI__) ou en mode démo dans un navigateur.
"use strict";

let VERSION = "0.1.0"; // remplacé au démarrage par la version de l'application
const $ = (id) => document.getElementById(id);
const store = {
  get(k, d) { try { const v = localStorage.getItem("lugus." + k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem("lugus." + k, JSON.stringify(v)); } catch {} },
};

// ------------------------------------------------------------------ Langue

let LANG = store.get("lang", null) || ((navigator.language || "fr").slice(0, 2));
if (!I18N[LANG]) LANG = "fr";
const t = (k, vars) => {
  let s = (I18N[LANG] && I18N[LANG][k]) ?? I18N.fr[k] ?? k;
  if (vars) for (const [a, b] of Object.entries(vars)) s = s.split("{" + a + "}").join(b);
  return s;
};
const num = (x) => String(x).replace(".", t("dec"));
const fmtBytes = (b) => {
  if (b == null) return "-";
  const u = t("units"); let i = 0, v = b;
  while (v >= 1000 && i < u.length - 1) { v /= 1000; i++; }
  return num(i ? v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2) : v) + " " + u[i];
};
const fmtTime = (s) => {
  if (!isFinite(s) || s < 0) return "-";
  s = Math.round(s);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return h ? `${h}h${String(m).padStart(2, "0")}` : `${m}:${String(r).padStart(2, "0")}`;
};
const pctOf = (a, b) => (b ? Math.min(100, Math.floor((a * 100) / b)) : 100);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const pathLabel = (p) => t("path_" + (["local", "direct", "relay"].includes(p) ? p : "unknown"));

function applyLang() {
  document.documentElement.lang = LANG;
  document.querySelectorAll("[data-i18n]").forEach((el) => (el.textContent = t(el.dataset.i18n)));
  document.querySelectorAll("[data-i18n-ph]").forEach((el) => (el.placeholder = t(el.dataset.i18nPh)));
  document.querySelectorAll("[data-i18n-title]").forEach((el) => (el.title = t(el.dataset.i18nTitle)));
  document.querySelectorAll("#lang button").forEach((b) => b.classList.toggle("on", b.dataset.l === LANG));
  renderDrop(); renderDest(); renderGo(); renderIdle(); renderSteps(); renderPhase(); renderPub(); checkInputs();
  $("vLabel").textContent = t(S.mode === "send" ? "verified_remote" : "verified_local");
  S.modalFn?.();
}
function setLang(l) { LANG = l; store.set("lang", l); applyLang(); }

// ------------------------------------------------------------------ Moteurs

function tauriBackend() {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  return {
    demo: false,
    pickFolder: (start) => invoke("pick_folder", { start }),
    pickFile: () => invoke("pick_file"),
    scan: (path) => invoke("scan_path", { path }),
    checkCode: (code) => invoke("check_code", { code }),
    checkLink: (link) => invoke("check_link", { link }),
    qr: (text) => invoke("qr_svg", { text }),
    send: (options) => invoke("start_send", { options }),
    receive: (options) => invoke("start_receive", { options }),
    newCode: () => invoke("new_code"),
    cancel: () => invoke("cancel"),
    open: (path) => invoke("open_path", { path }),
    reveal: (path) => invoke("reveal_path", { path }),
    onEvent: (cb) => listen("lugus", (e) => cb(e.payload)),
    onDrop: (cb) => {
      const wv = window.__TAURI__.webview?.getCurrentWebview?.();
      wv?.onDragDropEvent((e) => {
        const p = e.payload || {};
        if (p.type === "over" || p.type === "enter") $("drop").classList.add("over");
        else $("drop").classList.remove("over");
        if (p.type === "drop" && p.paths?.length) cb(p.paths[0]);
      });
    },
  };
}

// Simulation complète pour tester l'interface dans un navigateur.
function demoBackend() {
  let handler = () => {}, timers = [];
  const later = (ms, f) => timers.push(setTimeout(f, ms));
  const stopAll = () => { timers.forEach(clearTimeout); timers.forEach(clearInterval); timers = []; };
  const words = ["lanterne", "riviere", "falaise", "cheval", "sentier"];
  const code = () => `${String(Math.floor(Math.random() * 10000)).padStart(4, "0")}-${words[Math.floor(Math.random() * words.length)]}-${Math.floor(Math.random() * 10)}`;
  const link = "lugus://r/ng6bfcmwftaipe1e6x8tpdnhp5ijxqcixciyurigwfs3hg9ng58o?v=1";
  // Transfert simulé, puis vérification qui continue après 100 %.
  const run = (total, files, done) => {
    let moved = 0, checked = 0, i = 0;
    const tick = setInterval(() => {
      moved = Math.min(total, moved + total / 30);
      const cap = moved >= total ? total : moved * 0.8;
      checked = Math.min(cap, checked + (moved >= total ? total / 12 : total / 34));
      handler({ type: "progress", done: Math.round(moved), verified: Math.round(checked), total, path: "direct" });
      if (files && checked / total > (i + 1) / files.length && i < files.length) {
        handler({ type: "file_verified", path: files[i], size: Math.round(total / files.length), uncached: true }); i++;
      }
      if (checked >= total) { clearInterval(tick); done(); }
    }, 150);
    timers.push(tick);
  };
  const demoFiles = ["Proxy/A001C001.mov", "Proxy/A001C002.mov", "A001/A001C001.xml", "A001/A001C001.braw", "A001/A001C002.braw"];
  return {
    demo: true,
    pickFolder: async () => "/Volumes/RUSHES/J04",
    pickFile: async () => "/Volumes/RUSHES/A001_09291343_C003.braw",
    scan: async (path) => ({ path, name: path.split("/").pop(), files: path.endsWith(".braw") ? 1 : 214, bytes: path.endsWith(".braw") ? 4.2e9 : 6.2e11 }),
    checkCode: async (c) => { if (!/^\d{4}-[a-z]+-\d$/i.test(c.trim())) throw t("demo_code_shape"); return c.trim().toLowerCase(); },
    checkLink: async (l) => { if (!/^lugus:\/\/r\/[a-z0-9]{52}/.test(l.trim())) throw t("demo_link_shape"); },
    qr: async () => '<svg viewBox="0 0 21 21" xmlns="http://www.w3.org/2000/svg"><rect width="21" height="21" fill="#141619"/><path d="M1 1h7v7H1zM13 1h7v7h-7zM1 13h7v7H1z" fill="none" stroke="#e7e9ec"/><path d="M10 10h2v2h-2zM14 14h2v2h-2zM17 11h2v2h-2z" fill="#e7e9ec"/></svg>',
    send: async (o) => {
      later(100, () => handler({ type: "preparing" }));
      later(700, () => handler({ type: "ready", link, code: code(), name: o.path.split("/").pop(), files: 214, bytes: 6.2e11, relay: true }));
      later(2600, () => handler({ type: "published", seconds: 1.9 }));
      later(4200, () => handler({ type: "incoming", path: "direct" }));
      later(4700, () => handler({ type: "wrong_code", remaining: 2 }));
      later(6500, () => handler({ type: "incoming", path: "direct" }));
      later(7000, () => handler({ type: "authenticated", path: "direct" }));
      later(7400, () => { handler({ type: "selected", files: 214, bytes: 6.2e11 }); run(6.2e11, null, () => handler({ type: "delivered", files: 214, bytes: 6.2e11, receipt: "3f9a1c0e7b52d4a8", path: "direct" })); });
    },
    receive: async (o) => {
      const d = o.start_at ? 3000 : 0;
      if (o.start_at) handler({ type: "waiting", until: o.start_at, seconds: 180 });
      later(d + 300, () => handler({ type: "searching" }));
      later(d + 1800, () => handler({ type: "found", seconds: 1.5, path: "direct" }));
      later(d + 2500, () => {
        handler({ type: "manifest", name: "J04", files: demoFiles.length, bytes: 2.1e10, originals: 2, proxies: 2, sidecars: 1 });
        run(2.1e10, demoFiles, () => {
          handler({ type: "finalizing", step: "mhl" });
          later(900, () => handler({ type: "finalizing", step: "receipt" }));
          later(1600, () => handler({ type: "received", files: demoFiles.length, bytes: 2.1e10, seconds: 412, mhl: "/Reception/J04/J04_lugus_2026-11-14_221500.mhl", folder: "/Reception/J04", path: "direct" }));
        });
      });
    },
    newCode: async () => later(200, () => handler({ type: "new_code", code: code() })),
    cancel: async () => stopAll(),
    open: async () => log(t("demo_open"), "dim"),
    reveal: async () => log(t("demo_reveal"), "dim"),
    onEvent: (cb) => (handler = cb),
    onDrop: () => {},
  };
}

const api = window.__TAURI__ ? tauriBackend() : demoBackend();

// ------------------------------------------------------------------ État

const S = {
  mode: store.get("mode", "send"),
  running: false,
  source: null,                       // { path, name, files, bytes }
  dest: store.get("dest", null),
  linkOk: false, codeOk: false,
  total: 0, startT: 0, samples: [],
  steps: [], stepIdx: -1, final: null, // final : "ok" | "bad" | null
  phase: null,                        // { key, vars, sub, kind }
  pub: null,                          // "pending" | "online" | "slow" | null
  result: null,                       // { folder, mhl }
  modalFn: null,
};

const STEPS = {
  send: ["prepare", "publish", "wait", "code", "transfer", "verify_remote", "done"],
  recv: ["search", "code", "transfer", "verify", "mhl", "done"],
};

function setChip(kind, key) {
  const c = $("chip");
  c.className = "chip" + (kind ? " " + kind : "");
  c.dataset.i18n = key; c.textContent = t(key);
}

function setMode(mode) {
  if (S.running) return;
  S.mode = mode; store.set("mode", mode);
  $("modeSend").classList.toggle("on", mode === "send");
  $("modeRecv").classList.toggle("on", mode === "recv");
  $("sendForm").classList.toggle("hidden", mode !== "send");
  $("recvForm").classList.toggle("hidden", mode !== "recv");
  resetRight();
  renderGo();
}

// ------------------------------------------------------------------ Étapes et phrase d'état

function buildSteps(night) {
  S.steps = S.mode === "send" ? STEPS.send : night ? ["night", ...STEPS.recv] : STEPS.recv;
  S.stepIdx = -1; S.final = null;
  renderSteps();
}

// N'avance que vers l'avant : un événement tardif ne fait pas reculer la frise.
function step(key) {
  const i = S.steps.indexOf(key);
  if (i > S.stepIdx) { S.stepIdx = i; renderSteps(); }
}

function renderSteps() {
  $("steps").innerHTML = S.steps.map((k, i) => {
    let cls = "", mark = String(i + 1);
    if (i < S.stepIdx) { cls = "done"; mark = "✓"; }
    else if (i === S.stepIdx) {
      cls = S.final === "ok" ? "ok" : S.final === "bad" ? "bad" : "active";
      mark = S.final === "ok" ? "✓" : S.final === "bad" ? "✕" : mark;
    }
    return `<div class="step ${cls}"><i>${mark}</i>${esc(t("step_" + k))}</div>`;
  }).join("");
}

function phase(key, vars, sub, kind) { S.phase = { key, vars, sub, kind }; renderPhase(); }
function renderPhase() {
  const p = S.phase;
  if (!p) return;
  $("phase").className = "phase" + (p.kind ? " " + p.kind : "");
  $("phaseText").textContent = t(p.key, p.vars);
  $("phaseSub").textContent = p.sub ? (p.sub.raw ?? t(p.sub.key, p.sub.vars)) : "";
}

function setPub(state) { S.pub = state; renderPub(); }
function renderPub() {
  const b = $("pub");
  b.classList.toggle("hidden", !S.pub);
  if (!S.pub) return;
  b.className = "pub " + S.pub;
  b.textContent = t("pub_" + S.pub);
}

// ------------------------------------------------------------------ Formulaires

function renderDrop() {
  const d = $("drop");
  if (!S.source) {
    d.classList.remove("has");
    d.innerHTML = `<div class="empty"><b>${esc(t("drop_title"))}</b>${esc(t("drop_sub"))}</div>`;
    return;
  }
  d.classList.add("has");
  d.innerHTML = `<div class="path">${esc(S.source.path)}</div>
    <div class="stats"><span>${esc(t("files_word"))} <b>${S.source.files}</b></span><span>${esc(t("size_word"))} <b>${fmtBytes(S.source.bytes)}</b></span></div>`;
}

async function useSource(path) {
  if (!path || S.running) return;
  try {
    S.source = await api.scan(path);
    store.set("lastSource", path);
  } catch (e) {
    S.source = null;
    log(t("read_fail") + " : " + e, "err");
  }
  renderDrop(); renderGo();
}

function renderDest() {
  $("dest").textContent = S.dest || t("no_dest");
  $("dest").style.color = S.dest ? "" : "var(--dim)";
}

let checkTimer = null;
function checkInputs() {
  clearTimeout(checkTimer);
  checkTimer = setTimeout(async () => {
    const link = $("link").value.trim(), code = $("code").value.trim();
    S.linkOk = false; S.codeOk = false;
    if (link) {
      try { await api.checkLink(link); S.linkOk = true; hint("linkHint", t("link_ok"), "good"); }
      catch (e) { hint("linkHint", String(e), "bad"); }
    } else hint("linkHint", "");
    if (code) {
      try { await api.checkCode(code); S.codeOk = true; hint("codeHint", t("code_ok"), "good"); }
      catch (e) { hint("codeHint", t("code_typo") + " " + e, "bad"); }
    } else hint("codeHint", t("code_local"));
    renderGo();
  }, 200);
}
function hint(id, text, kind) { const h = $(id); h.textContent = text; h.className = "hint" + (kind ? " " + kind : ""); }

function renderNight() {
  $("night").classList.toggle("on", $("nightOn").checked);
}

function renderGo() {
  const go = $("go");
  if (S.mode === "send") {
    go.textContent = t("create_link");
    go.disabled = S.running || !S.source;
  } else {
    go.textContent = $("nightOn")?.checked ? t("receive_night") : t("receive_now");
    go.disabled = S.running || !S.linkOk || !S.codeOk || !S.dest;
  }
  go.classList.toggle("hidden", S.running);
  $("stop").classList.toggle("hidden", !S.running);
  $("modeSend").disabled = S.running; $("modeRecv").disabled = S.running;
}

// ------------------------------------------------------------------ Panneau de droite

function renderIdle() {
  const key = S.mode === "send" ? "idle_send" : "idle_recv";
  $("idle").innerHTML = `<b>${esc(t(key + "_title"))}</b>${esc(t(key))}`;
}

function resetRight() {
  ["status", "share", "banner", "prog", "cols"].forEach((id) => $(id).classList.add("hidden"));
  $("idle").classList.remove("hidden");
  $("log").innerHTML = ""; $("files").innerHTML = "";
  $("prog").classList.remove("done");
  $("share").classList.remove("locked");
  S.total = 0; S.samples = []; S.result = null; S.phase = null; S.pub = null;
  S.steps = []; S.stepIdx = -1; S.final = null;
  renderIdle(); renderSteps(); renderPub();
  $("vLabel").textContent = t(S.mode === "send" ? "verified_remote" : "verified_local");
  setChip("", "ready");
}

function showWork() {
  $("idle").classList.add("hidden");
  $("status").classList.remove("hidden");
  $("cols").classList.remove("hidden");
  $("files").classList.toggle("hidden", S.mode !== "recv");
  $("cols").style.gridTemplateColumns = S.mode === "recv" ? "1fr 1fr" : "1fr";
}

function log(text, kind) {
  const el = document.createElement("div");
  if (kind) el.className = kind;
  const now = new Date();
  el.innerHTML = `<i>${now.toLocaleTimeString(LANG, { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" })}</i><span>${esc(text)}</span>`;
  $("log").prepend(el);
}

async function showShare(link, code) {
  $("share").classList.remove("hidden");
  if (link) {
    $("shareLink").textContent = link;
    try { $("qr").innerHTML = await api.qr(link); } catch { $("qr").innerHTML = ""; }
  }
  if (code) $("shareCode").textContent = code;
}

function startProgress(total) {
  S.total = total; S.startT = Date.now(); S.samples = [];
  $("prog").classList.remove("hidden", "done");
  progress({ done: 0, verified: 0, total, path: "unknown" });
}

function progress(e) {
  const now = Date.now();
  const verified = e.verified ?? 0;
  S.samples.push([now, e.done]);
  while (S.samples.length > 2 && now - S.samples[0][0] > 6000) S.samples.shift();
  const [t0, d0] = S.samples[0];
  const rate = now > t0 ? ((e.done - d0) * 1000) / (now - t0) : 0;
  const moved = pctOf(e.done, e.total), checked = pctOf(verified, e.total);
  const allMoved = e.done >= e.total;

  $("pct").textContent = num(moved) + " %";
  $("amount").textContent = `${fmtBytes(e.done)} / ${fmtBytes(e.total)}`;
  $("bar").style.width = moved + "%";
  $("vbar").style.width = checked + "%";
  $("vpct").textContent = num(checked) + " %";
  $("speed").textContent = !allMoved && rate > 0 ? fmtBytes(rate) + "/s" : "-";
  $("eta").textContent = !allMoved && rate > 0 ? fmtTime((e.total - e.done) / rate) : "-";
  $("elapsed").textContent = fmtTime((now - S.startT) / 1000);
  $("route").textContent = pathLabel(e.path);

  // Phrase d'état pendant le transfert et la vérification.
  if (!S.running || S.final) return;
  if (S.mode === "send") {
    if (allMoved) {
      step("verify_remote");
      phase("ph_remote_verify", { p: num(checked) }, { key: "ph_remote_verify_sub" });
    } else {
      phase("ph_sending", { p: num(moved) }, { key: "ph_sending_sub", vars: { v: num(checked) } });
    }
  } else if (allMoved && verified < e.total) {
    step("verify");
    phase("ph_local_verify", { p: num(checked) }, { key: "ph_local_verify_sub" });
  } else if (!allMoved) {
    phase("ph_receiving", { p: num(moved) }, { key: "ph_receiving_sub", vars: { v: num(checked) } });
  }
}

function addFile(e) {
  const el = document.createElement("div");
  el.innerHTML = `<span class="tick">✓</span><span class="p" title="${esc(e.path)}">${esc(e.path)}</span><span class="s">${fmtBytes(e.size)}</span>`;
  $("files").prepend(el);
}

function banner(ok, title, sub, actions) {
  const b = $("banner");
  b.className = "banner " + (ok ? "ok" : "err");
  b.innerHTML = `${esc(title)}${sub ? `<small>${esc(sub)}</small>` : ""}${actions ? `<div class="row">${actions}</div>` : ""}`;
}

function finish(ok, title, sub, actions) {
  S.running = false;
  if (ok) step("done");
  S.final = ok ? "ok" : "bad";
  renderSteps();
  banner(ok, title, sub, actions);
  if (ok) {
    $("prog").classList.add("done");
    $("bar").style.width = "100%"; $("vbar").style.width = "100%"; $("vpct").textContent = "100 %";
  }
  $("share").classList.add("hidden");
  setChip(ok ? "ok" : "err", ok ? "chip_done" : "chip_error");
  renderGo();
}

// ------------------------------------------------------------------ Événements du moteur

function onEvent(e) {
  if (!S.running && e.type !== "error") return;
  switch (e.type) {
    // --- Envoi
    case "preparing":
      step("prepare"); phase("ph_preparing");
      break;
    case "ready":
      step("publish");
      showShare(e.link, e.code); setPub("pending");
      phase("ph_publishing", null, { key: "ph_publishing_sub" });
      log(t("ev_ready", { name: e.name, n: e.files, size: fmtBytes(e.bytes) }));
      if (!e.relay) log(t("ev_no_relay"), "warn");
      setChip("wait", "chip_publishing");
      break;
    case "published":
      setPub("online"); step("wait");
      if (S.stepIdx <= S.steps.indexOf("wait")) phase("ph_online", null, { key: "ph_online_sub" });
      log(t("ev_published", { s: num(Math.round(e.seconds)) }));
      setChip("wait", "chip_waiting");
      break;
    case "publish_slow":
      setPub("slow"); step("wait");
      if (S.stepIdx <= S.steps.indexOf("wait")) phase("ph_publish_slow", null, { key: "ph_publish_slow_sub" });
      log(t("ev_publish_slow"), "warn");
      setChip("wait", "chip_waiting");
      break;
    case "new_code":
      $("share").classList.remove("locked");
      showShare(null, e.code); log(t("ev_new_code"));
      phase("ph_new_code", null, { key: "ph_online_sub" });
      setChip("wait", "chip_waiting");
      break;
    case "incoming":
      step("wait");
      phase("ph_incoming", { path: pathLabel(e.path) });
      log(t("ev_incoming", { path: pathLabel(e.path) }));
      break;
    case "wrong_code":
      phase("ph_wrong_code", { n: e.remaining }, { key: "ph_online_sub" }, "");
      log(t("ev_wrong_code", { n: e.remaining }), "warn");
      break;
    case "locked":
      $("share").classList.add("locked");
      phase("ph_locked", null, null, "bad");
      log(t("ev_locked"), "err"); setChip("err", "chip_locked");
      break;
    case "authenticated":
      step("code");
      $("share").classList.add("hidden");
      phase("ph_code_ok_send");
      log(t("ev_authenticated", { path: pathLabel(e.path) }));
      setChip("run", "chip_sending");
      break;
    case "selected":
      step("transfer");
      log(t("ev_selected", { n: e.files, size: fmtBytes(e.bytes) }));
      startProgress(e.bytes);
      break;
    case "interrupted":
      $("share").classList.remove("hidden");
      phase("ph_interrupted", null, { key: "ph_interrupted_sub" });
      log(t("ev_interrupted"), "warn"); log(e.message, "dim");
      setChip("wait", "chip_waiting");
      break;
    case "delivered":
      phase("ph_delivered", null, null, "ok");
      finish(true, t("banner_delivered"), t("banner_delivered_sub", { n: e.files, size: fmtBytes(e.bytes), path: pathLabel(e.path) }));
      log(t("ev_receipt", { hash: e.receipt.slice(0, 16) }), "dim");
      break;
    case "expired":
      phase("ph_expired", null, null, "bad");
      finish(false, t("banner_expired"), t("banner_expired_sub"));
      break;

    // --- Réception
    case "waiting":
      step("night");
      phase("ph_night", { until: e.until }, { key: "ph_night_sub" });
      log(t("ev_waiting", { until: e.until, min: Math.ceil(e.seconds / 60) }));
      setChip("night", "chip_night");
      break;
    case "searching":
      step("search");
      phase("ph_searching", null, { key: "ph_searching_sub" });
      log(t("ev_searching")); setChip("run", "chip_searching");
      break;
    case "found":
      step("code");
      phase("ph_found", { path: pathLabel(e.path) });
      log(t("ev_found", { s: num(e.seconds.toFixed(1)), path: pathLabel(e.path) }));
      break;
    case "manifest":
      step("transfer");
      log(t("ev_code_ok"));
      log(t("ev_manifest", { name: e.name, n: e.files, size: fmtBytes(e.bytes), o: e.originals, p: e.proxies, s: e.sidecars }));
      setChip("run", "chip_receiving");
      startProgress(e.bytes);
      break;
    case "file_verified": addFile(e); break;
    case "finalizing":
      if (e.step === "mhl") { step("mhl"); phase("ph_mhl"); log(t("ev_all_verified")); }
      else phase("ph_receipt");
      break;
    case "received":
      S.result = { folder: e.folder, mhl: e.mhl };
      phase("ph_received", null, null, "ok");
      finish(true, t("banner_received"), t("banner_received_sub", { n: e.files, size: fmtBytes(e.bytes), d: fmtTime(e.seconds), path: pathLabel(e.path) }),
        `<button class="btn2" id="openFolder">${esc(t("open_folder"))}</button><button class="btn2" id="showMhl">${esc(t("show_mhl"))}</button>`);
      $("openFolder").onclick = () => api.open(S.result.folder);
      $("showMhl").onclick = () => api.reveal(S.result.mhl);
      log(t("ev_mhl", { path: e.mhl }), "dim");
      break;

    // --- Commun
    case "progress": progress(e); break;
    case "end_action": log(t("ev_end_" + e.action)); break;
    case "error":
      if (!S.running) { log(e.message, "err"); break; }
      phase("ph_error", null, { raw: e.message }, "bad");
      finish(false, t("banner_error"), e.message);
      log(e.message, "err");
      break;
  }
}

// ------------------------------------------------------------------ Actions

async function go() {
  if (S.running) return;
  resetRight();
  const night = S.mode === "recv" && $("nightOn").checked;
  buildSteps(night);
  showWork();
  S.running = true; renderGo();
  try {
    if (S.mode === "send") {
      step("prepare"); phase("ph_preparing");
      setChip("run", "chip_preparing");
      await api.send({ path: S.source.path, end_action: $("sendAfter").value });
    } else {
      step(night ? "night" : "search");
      phase(night ? "ph_night_start" : "ph_searching", null, night ? null : { key: "ph_searching_sub" });
      setChip("run", "chip_searching");
      await api.receive({
        link: $("link").value.trim(),
        code: $("code").value.trim(),
        dest: S.dest,
        start_at: night ? $("startAt").value : null,
        end_action: night ? $("recvAfter").value : "nothing",
      });
    }
  } catch (e) {
    phase("ph_error", null, { raw: String(e) }, "bad");
    finish(false, t("banner_error"), String(e));
  }
}

async function stop() {
  await api.cancel();
  S.running = false;
  S.final = "bad"; renderSteps();
  const key = S.mode === "recv" ? "ph_cancel_recv" : "ph_cancel_send";
  phase(key, null, null, "idle");
  log(t(key), "warn");
  setChip("", "chip_cancelled");
  $("share").classList.add("hidden");
  renderGo();
}

async function copy(text, button) {
  try { await navigator.clipboard.writeText(text); }
  catch {
    const area = document.createElement("textarea");
    area.value = text; document.body.appendChild(area); area.select();
    document.execCommand("copy"); area.remove();
  }
  const old = button.textContent;
  button.textContent = t("copied");
  setTimeout(() => (button.textContent = old), 1200);
}

// ------------------------------------------------------------------ Fenêtres

function modal(fn) {
  S.modalFn = fn;
  fn();
  $("modal").classList.remove("hidden");
}
function closeModal() { S.modalFn = null; $("modal").classList.add("hidden"); }

function aboutBox(welcome) {
  return () => {
    $("modalBox").innerHTML = `
      <h2>${esc(welcome ? t("welcome") : "Lugus Transfer")}</h2>
      <p class="by">by Domingo Olmo Martin</p>
      <p class="ver">v${esc(VERSION)}</p>
      <p style="margin-top:14px">${esc(t("about_desc"))}</p>
      <p>${esc(t("about_name"))}</p>
      <h4>${esc(t("license_title"))}</h4><p>${esc(t("license_text"))}</p>
      <h4>${esc(t("disclaimer_title"))}</h4>
      <div class="warnbox"><p>${esc(t("disclaimer_text"))}</p><p><b>${esc(t("disclaimer_rule"))}</b></p></div>
      <div class="acts"><button class="go" id="modalOk">${esc(welcome ? t("accept") : t("close"))}</button></div>`;
    $("modalOk").onclick = () => { if (welcome) store.set("accepted", true); closeModal(); };
  };
}

// ------------------------------------------------------------------ Démarrage

function init() {
  window.__TAURI__?.app?.getVersion?.().then((v) => { VERSION = v; S.modalFn?.(); }).catch(() => {});
  if (api.demo) $("chip").insertAdjacentHTML("afterend", '<span class="demo" data-i18n="demo"></span>');

  document.querySelectorAll("#lang button").forEach((b) => (b.onclick = () => setLang(b.dataset.l)));
  $("about").onclick = () => modal(aboutBox(false));
  $("modeSend").onclick = () => setMode("send");
  $("modeRecv").onclick = () => setMode("recv");

  $("drop").onclick = async () => useSource(await api.pickFolder(store.get("lastSource", null)));
  $("pickFile").onclick = async () => useSource(await api.pickFile());
  api.onDrop((path) => { if (S.mode !== "send") setMode("send"); useSource(path); });

  $("pickDest").onclick = async () => {
    const p = await api.pickFolder(S.dest);
    if (p) { S.dest = p; store.set("dest", p); renderDest(); renderGo(); }
  };
  $("link").oninput = checkInputs;
  $("code").oninput = checkInputs;
  $("link").addEventListener("paste", () => setTimeout(checkInputs, 0));

  $("nightOn").checked = store.get("nightOn", false);
  $("startAt").value = store.get("startAt", "01:00");
  $("recvAfter").value = store.get("recvAfter", "nothing");
  $("sendAfter").value = store.get("sendAfter", "nothing");
  $("nightOn").onchange = () => { store.set("nightOn", $("nightOn").checked); renderNight(); renderGo(); };
  $("startAt").onchange = () => store.set("startAt", $("startAt").value);
  $("recvAfter").onchange = () => store.set("recvAfter", $("recvAfter").value);
  $("sendAfter").onchange = () => store.set("sendAfter", $("sendAfter").value);

  $("go").onclick = go;
  $("stop").onclick = stop;
  $("copyLink").onclick = (ev) => copy($("shareLink").textContent, ev.target);
  $("copyCode").onclick = (ev) => copy($("shareCode").textContent, ev.target);
  $("newCode").onclick = () => api.newCode().catch((e) => log(String(e), "err"));

  api.onEvent(onEvent);
  applyLang();
  renderNight();
  setMode(S.mode);
  if (!store.get("accepted", false)) modal(aboutBox(true));
}

init();

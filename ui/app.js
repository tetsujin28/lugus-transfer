// Lugus Transfer by Domingo Olmo Martin : interface.
// Fonctionne dans l'app Tauri (window.__TAURI__) ou en mode démo dans un navigateur.
"use strict";

let VERSION = "0.3.0"; // remplacé au démarrage par la version de l'application
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
  renderDrop(); renderDest(); renderGo(); renderIdle(); renderSteps(); renderPhase(); renderPub(); renderPicker(); checkInputs();
  $("vLabel").textContent = t(S.mode === "send" ? "verified_remote" : "verified_local");
  S.modalFn?.();
}
function setLang(l) { LANG = l; store.set("lang", l); applyLang(); renderVersion(); }

// ------------------------------------------------------------------ Erreurs du moteur
// Le moteur écrit ses erreurs en français. Les cas connus sont reconnus ici et
// affichés dans la langue de l'interface ; le texte d'origine va dans le journal.

const ENGINE_ERRORS = [
  [/émetteur introuvable/, "err_not_found"],
  [/code incorrect, plus aucun essai/, "err_code_none_left"],
  [/code incorrect, (\d+) essai/, "err_code_left", (m) => ({ n: m[1] })],
  [/trop tôt après un essai raté, réessayez dans (\d+) s/, "err_too_soon", (m) => ({ s: m[1] })],
  [/ce code est bloqué/, "err_locked"],
  [/version incompatible/, "err_version"],
  [/ce lien a expiré/, "err_expired"],
  [/l'expéditeur est occupé/, "err_busy"],
  [/(\S[^:]*?) : refusé par l'émetteur \(SourceChanged\)|un fichier a changé chez l'expéditeur/, "err_source_changed", (m) => ({ path: m[1] || "" })],
  [/(\S[^:]*?) : refusé par l'émetteur \(FileUnavailable\)|un fichier est illisible chez l'expéditeur/, "err_file_unavailable", (m) => ({ path: m[1] || "" })],
  [/(\S.*?) existe déjà avec une taille différente/, "err_exists", (m) => ({ path: m[1] })],
  [/existe déjà à destination mais diffère de la source/, "err_exists", (m) => ({ path: "" })],
  [/vérification de ([^:]+?): .*hash du fichier différent de la source/, "err_hash", (m) => ({ path: m[1] })],
  [/hash du fichier différent de la source|bloc altéré/, "err_hash", () => ({ path: "" })],
  [/code invalide \(vérifié en local/, "err_code_invalid"],
  [/lien invalide/, "err_link_invalid"],
  [/l'émetteur n'a pas prouvé qu'il connaît le code/, "err_sender_proof"],
  [/la machine jointe n'est pas celle du lien/, "err_wrong_machine"],
  [/aucun fichier à envoyer/, "err_nothing_to_send"],
  [/des fichiers choisis ont disparu/, "err_files_gone"],
  [/aucun fichier choisi/, "err_nothing_chosen"],
  [/connexion perdue|connection lost|timed out|reset by peer|closed by peer/i, "err_connection_lost"],
];

function engineError(raw) {
  const msg = String(raw ?? "");
  for (const [re, key, vars] of ENGINE_ERRORS) {
    const m = msg.match(re);
    if (m) return { text: t(key, vars ? vars(m) : null).replace(" : .", ".").replace(": .", "."), raw: msg, known: true };
  }
  return { text: msg, raw: msg, known: false };
}

// ------------------------------------------------------------------ Moteurs

function tauriBackend() {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  return {
    demo: false,
    pickFolder: (start) => invoke("pick_folder", { start }),
    pickDest: (start) => invoke("pick_folder", { start }),
    pickFiles: () => invoke("pick_files"),
    scan: (paths) => invoke("scan_paths", { paths }),
    checkCode: (code) => invoke("check_code", { code }),
    checkLink: (link) => invoke("check_link", { link }),
    send: (options) => invoke("start_send", { options }),
    receive: (options) => invoke("start_receive", { options }),
    newCode: () => invoke("new_code"),
    choose: (files) => invoke("choose_files", { files }),
    cancel: () => invoke("cancel"),
    open: (path) => invoke("open_path", { path }),
    reveal: (path) => invoke("reveal_path", { path }),
    openReleases: (url) => invoke("open_release_page", { url }),
    onEvent: (cb) => listen("lugus", (e) => cb(e.payload)),
    onDrop: (cb) => {
      const wv = window.__TAURI__.webview?.getCurrentWebview?.();
      wv?.onDragDropEvent((e) => {
        const p = e.payload || {};
        if (p.type === "over" || p.type === "enter") $("drop").classList.add("over");
        else $("drop").classList.remove("over");
        if (p.type === "drop" && p.paths?.length) cb(p.paths);
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
  const code = () => window.LUGUS_DEMO_CODE || `${String(Math.floor(Math.random() * 10000)).padStart(4, "0")}-${words[Math.floor(Math.random() * words.length)]}-${Math.floor(Math.random() * 10)}`;
  const link = "lugus://r/ng6bfcmwftaipe1e6x8tpdnhp5ijxqcixciyurigwfs3hg9ng58o?v=1";
  // Transfert simulé, puis vérification qui continue après 100 %.
  // Débit simulé : rapide par défaut, réaliste si window.LUGUS_DEMO_RATE (octets/s)
  // est fixé (captures du mode d'emploi).
  const run = (total, files, done) => {
    let moved = 0, checked = 0, i = 0;
    const rate = window.LUGUS_DEMO_RATE;
    const step = rate ? rate * 0.15 : total / 30;
    const tick = setInterval(() => {
      moved = Math.min(total, moved + step);
      const cap = moved >= total ? total : moved * 0.8;
      checked = Math.min(cap, checked + (moved >= total ? (rate ? step * 2.5 : total / 12) : (rate ? step * 0.9 : total / 34)));
      handler({ type: "progress", done: Math.round(moved), verified: Math.round(checked), total, path: "direct" });
      // Les fichiers sont annoncés vérifiés dans l'ordre, selon leur taille réelle.
      while (files && i < files.length && checked >= files.slice(0, i + 1).reduce((a, f) => a + f.size, 0)) {
        handler({ type: "file_verified", path: files[i].path, size: files[i].size, uncached: true }); i++;
      }
      if (checked >= total) { clearInterval(tick); done(); }
    }, 150);
    timers.push(tick);
  };
  // Contenu simulé d'une journée de tournage (« J04 » en français, « D04 » sinon).
  const day = () => (LANG === "fr" ? "J04" : "D04");
  const demoEntries = (() => {
    const out = [];
    for (const card of ["A001", "A002"]) {
      for (let c = 1; c <= 4; c++) {
        const clip = `${card}C00${c}`;
        out.push({ path: `${card}/${clip}.braw`, size: 2.6e10 + c * 3.1e9, kind: "Original" });
        out.push({ path: `${card}/${clip}.sidecar.xml`, size: 4200, kind: "Sidecar" });
        out.push({ path: `Proxy/${card}/${clip}_proxy.mov`, size: 6.4e8 + c * 9e7, kind: "Proxy" });
      }
    }
    out.push({ path: `${LANG === "fr" ? "Rapport" : LANG === "es" ? "Informe" : "Report"}_${day()}.pdf`, size: 2.4e5, kind: "Sidecar" });
    return out.sort((a, b) => (a.path < b.path ? -1 : 1));
  })();
  let chooseFn = null, folderCalls = 0;
  // Deuxième endroit simulé : une autre carte, avec ses propres fichiers.
  const demoExtra = [
    { path: "B001/B001C001.braw", size: 3.1e10, kind: "Original" },
    { path: "B001/B001C001.sidecar.xml", size: 4200, kind: "Sidecar" },
  ];
  const demoList = (path) => (path.includes("/CARTE_B") ? demoExtra : path.endsWith(".braw") ? [{ path: path.split("/").pop(), size: 4.2e9, kind: "Original" }] : demoEntries);
  return {
    demo: true,
    pickFolder: async () => (folderCalls++ === 0 ? `/Volumes/RUSHES/${day()}` : "/Volumes/CARTE_B/Rushes"),
    pickDest: async () => "/Volumes/MONTAGE/Rushes",
    pickFiles: async () => ["A001C001.braw", "A001C002.braw", "A001C001.sidecar.xml"].map((f) => `/Volumes/RUSHES/${day()}/A001/${f}`),
    scan: async (paths) => {
      const sum = (list) => list.reduce((a, e) => a + e.size, 0);
      const parent = (p) => p.split("/").slice(0, -1).join("/");
      const base = (p) => p.split("/").pop();
      const groups = [];
      for (const p of paths) {
        const g = groups.find((x) => x.root === parent(p));
        g ? g.items.push(p) : groups.push({ root: parent(p), items: [p] });
      }
      return groups.map(({ root, items }) => {
        if (items.length === 1) {
          const p = items[0], list = demoList(p), file = p.endsWith(".braw");
          return { path: p, root: file ? root : p, root_name: base(file ? root : p), name: base(p), files: list.length, bytes: sum(list), folder: !file, whole: !file, entries: list, only: null };
        }
        const names = items.map(base);
        const entries = demoEntries.filter((e) => e.path.startsWith("A001/") && names.includes(e.path.split("/").pop()))
          .map((e) => ({ ...e, path: e.path.split("/").pop() }));
        return { path: root, root, root_name: base(root), name: base(root), files: entries.length, bytes: sum(entries), folder: true, whole: false, entries, only: entries.map((e) => e.path) };
      });
    },
    checkCode: async (c) => { if (!/^\d{4}-[a-z]+-\d$/i.test(c.trim())) throw t("demo_code_shape"); return c.trim().toLowerCase(); },
    checkLink: async (l) => { if (!/^lugus:\/\/r\/[a-z0-9]{52}/.test(l.trim())) throw t("demo_link_shape"); },
    send: async (o) => {
      later(100, () => handler({ type: "preparing" }));
      const chosen = o.sources.flatMap((sp) => demoList(sp.path).filter((e) => !sp.only || sp.only.includes(e.path)));
      const files = chosen.length;
      const bytes = chosen.reduce((a, e) => a + e.size, 0);
      const name = o.sources.length > 1 ? `Rushes_${new Date().toISOString().slice(0, 10)}` : o.sources[0].path.split("/").pop();
      later(700, () => handler({ type: "ready", link, code: code(), name, files, bytes, relay: true }));
      later(2600, () => handler({ type: "published", seconds: 1.9 }));
      later(4200, () => handler({ type: "incoming", path: "direct" }));
      later(4700, () => handler({ type: "wrong_code", remaining: 2 }));
      later(6500, () => handler({ type: "incoming", path: "direct" }));
      later(7000, () => handler({ type: "authenticated", path: "direct" }));
      later(7400, () => { handler({ type: "selected", files, bytes }); run(bytes, null, () => handler({ type: "delivered", files, bytes, receipt: "3f9a1c0e7b52d4a8", path: "direct" })); });
    },
    receive: async (o) => {
      const d = o.start_at ? 3000 : 0;
      if (o.start_at) handler({ type: "waiting", until: o.start_at, seconds: 180 });
      later(d + 300, () => handler({ type: "searching" }));
      later(d + 1800, () => handler({ type: "found", seconds: 1.5, path: "direct" }));
      const count = (k) => demoEntries.filter((e) => e.kind === k).length;
      const all = demoEntries.reduce((a, e) => a + e.size, 0);
      const go = (picked) => {
        // Proxys, puis annexes, puis originaux, comme le vrai moteur.
        const rank = { Proxy: 0, Sidecar: 1, Original: 2 };
        const chosen = picked.map((i) => demoEntries[i]).sort((a, b) => rank[a.kind] - rank[b.kind]);
        const bytes = chosen.reduce((a, e) => a + e.size, 0);
        const started = Date.now();
        if (!o.start_at) handler({ type: "chosen", files: chosen.length, bytes });
        run(bytes, chosen, () => {
          handler({ type: "finalizing", step: "mhl" });
          later(900, () => handler({ type: "finalizing", step: "receipt" }));
          later(1600, () => handler({ type: "received", files: chosen.length, bytes, seconds: (Date.now() - started) / 1000, mhl: `/Volumes/MONTAGE/Rushes/${day()}/${day()}_lugus_2026-11-15_143212.mhl`, folder: `/Volumes/MONTAGE/Rushes/${day()}`, path: "direct" }));
        });
      };
      later(d + 2500, () => {
        handler({ type: "manifest", name: day(), files: demoEntries.length, bytes: all, originals: count("Original"), proxies: count("Proxy"), sidecars: count("Sidecar") });
        if (o.start_at) go(demoEntries.map((_, i) => i));
        else { chooseFn = go; handler({ type: "choose", files: demoEntries }); }
      });
    },
    newCode: async () => later(200, () => handler({ type: "new_code", code: code() })),
    choose: async (files) => { const f = chooseFn; chooseFn = null; f?.(files); },
    cancel: async () => stopAll(),
    open: async () => log(t("demo_open"), "dim"),
    reveal: async () => log(t("demo_reveal"), "dim"),
    openReleases: async () => log(t("demo_open"), "dim"),
    onEvent: (cb) => (handler = cb),
    onDrop: () => {},
  };
}

const api = window.__TAURI__ ? tauriBackend() : demoBackend();

// ------------------------------------------------------------------ État

const S = {
  mode: store.get("mode", "send"),
  running: false,
  sources: [],                        // [{ path, root, root_name, name, files, bytes, folder, whole, entries, only }]
  dest: store.get("dest", null),
  linkOk: false, codeOk: false,
  total: 0, startT: 0, samples: [],
  steps: [], stepIdx: -1, final: null, // final : "ok" | "bad" | null
  phase: null,                        // { key, vars, sub, kind }
  pub: null,                          // "pending" | "online" | "slow" | null
  result: null,                       // { folder, mhl }
  pick: null,                         // écran de choix : { side, entries, on: Set, open: Set }
  sendPick: null,                     // choix d'envoi mis de côté pendant la réception
  night: false,
  modalFn: null,
};

const STEPS = {
  send: ["prepare", "publish", "wait", "code", "transfer", "verify_remote", "done"],
  recv: ["search", "code", "choose", "transfer", "verify", "mhl", "done"],
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
  S.steps = S.mode === "send" ? STEPS.send
    : night ? ["night", ...STEPS.recv.filter((k) => k !== "choose")] : STEPS.recv;
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
  if (!S.sources.length) {
    d.classList.remove("has");
    d.innerHTML = `<div class="empty"><b>${esc(t("drop_title"))}</b>${esc(t("drop_sub"))}</div>`;
    return;
  }
  d.classList.add("has");
  const all = S.sources.reduce((a, x) => a + x.entries.length, 0);
  const allBytes = S.sources.reduce((a, x) => a + x.entries.reduce((b, e) => b + e.size, 0), 0);
  const partial = S.pick?.side === "send" && S.pick.on.size < S.pick.entries.length;
  const files = partial ? `${S.pick.on.size} / ${all}` : all;
  const bytes = partial ? S.pick.entries.filter((_, i) => S.pick.on.has(i)).reduce((a, e) => a + e.size, 0) : allBytes;
  const multi = S.sources.length > 1;
  const rows = S.sources.map((x, i) => `<div class="src"><div class="path">${esc(x.path)}</div>${!S.running && multi ? `<button class="rm" data-rm="${i}" title="${esc(t("remove_source_title"))}">✕</button>` : ""}</div>`).join("");
  d.innerHTML = `${S.running ? "" : `<button class="clear" id="clearSource" title="${esc(t("clear_source_title"))}">✕ ${esc(t(multi ? "clear_all" : "clear_source"))}</button>`}${rows}
    <div class="stats"><span>${esc(t("files_word"))} <b>${files}</b></span><span>${esc(t("size_word"))} <b>${fmtBytes(bytes)}</b></span></div>${multi ? `<div class="multi">${esc(t("multi_note"))}</div>` : ""}`;
}

// Vide ce qui a été choisi pour l'envoi et revient à l'état de départ.
function clearSource() {
  if (S.running) return;
  S.sources = []; S.sendPick = null;
  hint("srcHint", "");
  closePicker();
  resetRight();
  renderDrop(); renderGo();
}

// Nom de chaque source à l'arrivée : celui de son dossier ou de son disque, avec
// _2, _3… si deux sources portent le même nom (comme le fait le moteur).
function sourceNames() {
  const taken = [], names = [];
  for (const x of S.sources) {
    const base = x.root_name || x.root.split(/[\\/]/).filter(Boolean).pop() || x.name || "Rushes";
    let name = base, n = 2;
    while (taken.includes(name.toLowerCase())) name = `${base}_${n++}`;
    taken.push(name.toLowerCase()); names.push(name);
  }
  return names;
}

// Tous les fichiers à envoyer, toutes sources confondues. Avec plusieurs sources,
// le chemin affiché commence par le nom de la source, comme chez le destinataire.
function sendEntries() {
  const names = sourceNames(), multi = S.sources.length > 1, out = [];
  S.sources.forEach((x, si) => x.entries.forEach((e) => out.push({ ...e, rel: e.path, path: multi ? `${names[si]}/${e.path}` : e.path, src: si })));
  return out;
}

const entryKey = (e) => `${S.sources[e.src].root}\u0000${e.rel}`;

// Fichiers décochés dans l'écran de choix, pour les garder décochés après un ajout.
function offKeys() {
  const off = new Set();
  if (S.pick?.side === "send") S.pick.entries.forEach((e, i) => { if (!S.pick.on.has(i)) off.add(entryKey(e)); });
  return off;
}

// Reconstruit l'écran de choix de l'envoi après un ajout ou un retrait.
function syncSendPicker(off) {
  const entries = sendEntries();
  if (entries.length <= 1) { closePicker(); return; }
  const open = S.sources.length > 1 ? new Set(sourceNames().map((n) => n + "/")) : new Set();
  S.pick = { side: "send", entries, on: new Set(entries.map((_, i) => i).filter((i) => !off.has(entryKey(entries[i])))), open };
  $("idle").classList.add("hidden");
  $("picker").classList.remove("hidden");
  renderPicker();
}

// Deux ajouts du même dossier ne font qu'une source.
function mergeSource(a, b) {
  const seen = new Map();
  [...a.entries, ...b.entries].forEach((e) => seen.set(e.path, e));
  const entries = [...seen.values()].sort((x, y) => (x.path < y.path ? -1 : 1));
  const wanted = (x) => x.only ?? x.entries.map((e) => e.path);
  const whole = a.whole || b.whole;
  const only = whole ? null : [...new Set([...wanted(a), ...wanted(b)])].sort();
  return { ...a, path: a.root, folder: true, whole, only, entries, files: entries.length, bytes: entries.reduce((n, e) => n + e.size, 0) };
}

// `child` est-il dans le dossier `parent` ?
function inside(child, parent) {
  if (child === parent) return false;
  return /[\\/]$/.test(parent) ? child.startsWith(parent) : child.startsWith(parent + "/") || child.startsWith(parent + "\\");
}

// Ajoute les sources trouvées. Un dossier déjà envoyé en entier contient déjà ce
// qu'on y ajoute : on ne l'envoie pas deux fois. Renvoie la clé du message à
// montrer quand quelque chose a été ignoré ou absorbé pour cette raison.
function addSummaries(found) {
  let note = "";
  for (const n of found) {
    if (S.sources.some((x) => x.whole && inside(n.root, x.root))) { note = "already_included"; continue; }
    if (n.whole) {
      const kept = S.sources.filter((x) => !inside(x.root, n.root));
      if (kept.length < S.sources.length) note = "absorbed";
      S.sources = kept;
    }
    const k = S.sources.findIndex((x) => x.root === n.root);
    if (k < 0) S.sources.push(n); else S.sources[k] = mergeSource(S.sources[k], n);
  }
  return note;
}

// Ce qui est glissé ou choisi s'ajoute à ce qui est déjà là : dossiers, fichiers,
// d'un ou de plusieurs disques.
async function addSources(paths) {
  if (!paths || S.running) return;
  if (!Array.isArray(paths)) paths = [paths];
  if (!paths.length) return;
  let found;
  try {
    found = await api.scan(paths);
  } catch (e) {
    const msg = String(e);
    const text = msg.startsWith("lugus:") ? t(msg.slice(6)) : engineError(msg).known ? engineError(msg).text : t("read_fail") + " : " + msg;
    hint("srcHint", text, "bad");
    return;
  }
  try {
    const off = offKeys();
    const note = addSummaries(found);
    store.set("lastSource", found[found.length - 1].path);
    hint("srcHint", note ? t(note) : "");
    syncSendPicker(off);
  } catch (e) {
    hint("srcHint", t("read_fail") + " : " + e, "bad");
  }
  renderDrop(); renderGo();
}

function removeSource(i) {
  if (S.running) return;
  const off = offKeys();
  S.sources.splice(i, 1);
  if (!S.sources.length) return clearSource();
  syncSendPicker(off);
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
    go.disabled = S.running || !S.sources.length || (S.pick?.side === "send" && S.pick.on.size === 0);
  } else {
    go.textContent = $("nightOn")?.checked ? t("receive_night") : t("receive_now");
    go.disabled = S.running || !S.linkOk || !S.codeOk || !S.dest;
  }
  go.classList.toggle("hidden", S.running);
  if (S.mode === "send" && S.sources.length) $("clearSource")?.classList.toggle("hidden", S.running);
  $("stop").classList.toggle("hidden", !S.running);
  $("modeSend").disabled = S.running; $("modeRecv").disabled = S.running;
}

// ------------------------------------------------------------------ Choix des fichiers
// Même écran à l'envoi (avant de créer le lien) et à la réception (après le
// code) : arbre du dossier, tout coché, filtres par type de fichier.

const KINDS = [["Original", "kind_original"], ["Proxy", "kind_proxy"], ["Sidecar", "kind_sidecar"]];

function openPicker(side, entries) {
  S.pick = { side, entries, on: new Set(entries.map((_, i) => i)), open: new Set() };
  $("idle").classList.add("hidden");
  $("picker").classList.remove("hidden");
  renderPicker();
}
function closePicker() {
  S.pick = null;
  $("picker").classList.add("hidden");
  if (!S.running && $("status").classList.contains("hidden")) $("idle").classList.remove("hidden");
}

// Sources à envoyer, avec pour chacune les chemins choisis (null : tout part).
function sendSpecs() {
  const sending = S.pick?.side === "send";
  const entries = sending ? S.pick.entries : sendEntries();
  const specs = [];
  S.sources.forEach((x, si) => {
    const mine = entries.map((e, i) => ({ e, i })).filter(({ e }) => e.src === si);
    const chosen = mine.filter(({ i }) => !sending || S.pick.on.has(i)).map(({ e }) => e);
    if (!chosen.length) return;
    if (chosen.length === mine.length) specs.push({ path: x.path, only: x.only ?? null });
    else specs.push({ path: x.root, only: chosen.map((e) => e.rel) });
  });
  return specs;
}

// Arbre : { dirs: Map(nom -> nœud), files: [indice], all: [indices du sous-arbre] }
function pickTree() {
  const root = { dirs: new Map(), files: [], all: [] };
  S.pick.entries.forEach((e, i) => {
    const parts = e.path.split("/");
    let node = root; node.all.push(i);
    for (const part of parts.slice(0, -1)) {
      if (!node.dirs.has(part)) node.dirs.set(part, { dirs: new Map(), files: [], all: [] });
      node = node.dirs.get(part); node.all.push(i);
    }
    node.files.push(i);
  });
  return root;
}

function renderPicker() {
  const p = S.pick;
  if (!p) return;
  const recv = p.side === "recv";
  $("pickTitle").textContent = t(recv ? "pick_title_recv" : "pick_title_send");
  $("pickHint").textContent = t(recv ? "pick_hint_recv" : "pick_hint_send");

  // Filtres : tout, rien, puis un bouton par type présent.
  const state = (idx) => {
    const n = idx.filter((i) => p.on.has(i)).length;
    return n === 0 ? "" : n === idx.length ? "on" : "part";
  };
  let tools = `<button class="tool" data-act="all">${esc(t("pick_all"))}</button><button class="tool" data-act="none">${esc(t("pick_none"))}</button>`;
  for (const [kind, key] of KINDS) {
    const idx = p.entries.map((e, i) => (e.kind === kind ? i : -1)).filter((i) => i >= 0);
    if (idx.length) tools += `<button class="tool ${state(idx)}" data-kind="${kind}">${esc(t(key))}<em>${idx.length}</em></button>`;
  }
  $("pickTools").innerHTML = tools;

  // Arbre.
  const rows = [];
  const walk = (node, prefix, depth) => {
    for (const [name, child] of [...node.dirs].sort((a, b) => a[0].localeCompare(b[0]))) {
      const path = prefix + name + "/";
      const n = child.all.filter((i) => p.on.has(i)).length;
      const size = child.all.reduce((a, i) => a + p.entries[i].size, 0);
      const open = p.open.has(path);
      rows.push(`<div class="row dir${n ? "" : " off"}" data-dir="${esc(path)}" style="padding-left:${10 + depth * 18}px">
        <span class="tw">${open ? "▾" : "▸"}</span><input type="checkbox" data-mix="${n > 0 && n < child.all.length ? 1 : 0}" ${n ? "checked" : ""}>
        <span class="nm">${esc(name)} <span class="kd">${n} / ${child.all.length}</span></span><span class="sz">${fmtBytes(size)}</span></div>`);
      if (open) walk(child, path, depth + 1);
    }
    for (const i of node.files) {
      const e = p.entries[i], on = p.on.has(i);
      const kind = KINDS.find((k) => k[0] === e.kind);
      rows.push(`<div class="row${on ? "" : " off"}" data-i="${i}" style="padding-left:${10 + depth * 18}px">
        <span class="tw"></span><input type="checkbox" ${on ? "checked" : ""}>
        <span class="nm" title="${esc(e.path)}">${esc(e.path.split("/").pop())}${kind ? `<span class="kd">${esc(t(kind[1] + "_one"))}</span>` : ""}</span><span class="sz">${fmtBytes(e.size)}</span></div>`);
    }
  };
  S.pickRoot = pickTree();
  walk(S.pickRoot, "", 0);
  $("tree").innerHTML = rows.join("");
  $("tree").querySelectorAll('input[data-mix="1"]').forEach((c) => (c.indeterminate = true));

  // Résumé et bouton.
  const bytes = [...p.on].reduce((a, i) => a + p.entries[i].size, 0);
  $("pickSum").innerHTML = p.on.size
    ? t("pick_summary", { n: `<b>${p.on.size}</b>`, total: p.entries.length, size: `<b>${esc(fmtBytes(bytes))}</b>` })
    : esc(t("pick_empty"));
  $("pickOk").classList.toggle("hidden", !recv);
  $("pickOk").textContent = t("pick_confirm_recv");
  $("pickOk").disabled = p.on.size === 0;
  if (!recv) { renderDrop(); renderGo(); }
}

function pickSet(indices, on) {
  for (const i of indices) on ? S.pick.on.add(i) : S.pick.on.delete(i);
  renderPicker();
}

function dirNode(path) {
  let node = S.pickRoot;
  for (const part of path.split("/").filter(Boolean)) node = node.dirs.get(part);
  return node;
}

function initPicker() {
  $("pickTools").onclick = (ev) => {
    const b = ev.target.closest(".tool");
    if (!b || !S.pick) return;
    const all = S.pick.entries.map((_, i) => i);
    if (b.dataset.act === "all") return pickSet(all, true);
    if (b.dataset.act === "none") return pickSet(all, false);
    const idx = all.filter((i) => S.pick.entries[i].kind === b.dataset.kind);
    pickSet(idx, !idx.every((i) => S.pick.on.has(i)));
  };
  $("tree").onclick = (ev) => {
    const row = ev.target.closest(".row");
    if (!row || !S.pick) return;
    const box = ev.target.tagName === "INPUT";
    if (row.dataset.dir) {
      const path = row.dataset.dir;
      if (box) {
        const idx = dirNode(path).all;
        pickSet(idx, !idx.every((i) => S.pick.on.has(i)));
      } else {
        S.pick.open.has(path) ? S.pick.open.delete(path) : S.pick.open.add(path);
        renderPicker();
      }
    } else {
      const i = Number(row.dataset.i);
      pickSet([i], !S.pick.on.has(i));
    }
  };
  $("pickOk").onclick = async () => {
    if (!S.pick || S.pick.side !== "recv" || !S.pick.on.size) return;
    const files = [...S.pick.on].sort((a, b) => a - b);
    closePicker();
    phase("ph_chosen");
    try { await api.choose(files); }
    catch (e) { phase("ph_error", null, { raw: String(e) }, "bad"); finish(false, t("banner_error"), String(e)); }
  };
}

// ------------------------------------------------------------------ Panneau de droite

function renderIdle() {
  const key = S.mode === "send" ? "idle_send" : "idle_recv";
  $("idle").innerHTML = `<b>${esc(t(key + "_title"))}</b>${esc(t(key))}`;
}

function resetRight() {
  ["status", "share", "banner", "prog", "cols"].forEach((id) => $(id).classList.add("hidden"));
  // Le choix fait à l'envoi est gardé de côté quand on passe en réception.
  if (S.pick?.side === "send" && S.mode !== "send") S.sendPick = S.pick;
  if (S.pick && (S.pick.side === "recv" || S.mode !== "send")) closePicker();
  if (S.mode === "send" && !S.pick && S.sendPick && !S.running) {
    S.pick = S.sendPick; S.sendPick = null;
    $("picker").classList.remove("hidden"); renderPicker();
  }
  $("idle").classList.toggle("hidden", !!S.pick);
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
  $("picker").classList.add("hidden");
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
  // Les premiers instants montent d'un coup au point de reprise : ce n'est pas
  // du débit, la mesure démarre après.
  if (now - S.startT < 1500) S.samples = [];
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
  if (S.pick?.side === "recv") closePicker();
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
      log(t("ev_interrupted"), "warn");
      { const err = engineError(e.message); if (err.known) log(err.text, "warn"); log(err.raw, "dim"); }
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
    case "search_retry":
      phase("ph_search_retry", { n: e.attempt }, { key: "ph_searching_sub_retry" });
      if (e.attempt === 2 || e.attempt % 5 === 0) log(t("ev_search_retry", { n: e.attempt }), "dim");
      break;
    case "found":
      step("code");
      phase("ph_found", { path: pathLabel(e.path) });
      log(t("ev_found", { s: num(e.seconds.toFixed(1)), path: pathLabel(e.path) }));
      break;
    case "manifest":
      log(t("ev_code_ok"));
      log(t("ev_manifest", { name: e.name, n: e.files, size: fmtBytes(e.bytes), o: e.originals, p: e.proxies, s: e.sidecars }));
      // En mode Noctiluca, tout est reçu sans écran de choix.
      if (S.night) { step("transfer"); setChip("run", "chip_receiving"); startProgress(e.bytes); }
      break;
    case "choose":
      step("choose");
      phase("ph_choose", null, { key: "ph_choose_sub" });
      setChip("wait", "chip_choosing");
      openPicker("recv", e.files);
      break;
    case "chosen":
      step("transfer");
      log(t("ev_chosen", { n: e.files, size: fmtBytes(e.bytes) }));
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
    case "error": {
      const err = engineError(e.message);
      if (!S.running) { log(err.text, "err"); if (err.known) log(err.raw, "dim"); break; }
      phase("ph_error", null, { raw: err.text }, "bad");
      finish(false, t("banner_error"), err.text);
      log(err.text, "err");
      if (err.known) log(err.raw, "dim");
      break;
    }
  }
}

// ------------------------------------------------------------------ Actions

async function go() {
  if (S.running) return;
  const sources = S.mode === "send" ? sendSpecs() : null;
  resetRight();
  const night = S.mode === "recv" && $("nightOn").checked;
  S.night = night;
  buildSteps(night);
  showWork();
  S.running = true; renderGo();
  try {
    if (S.mode === "send") {
      step("prepare"); phase("ph_preparing");
      setChip("run", "chip_preparing");
      await api.send({ sources, end_action: $("sendAfter").value });
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
    const err = engineError(e);
    phase("ph_error", null, { raw: err.text }, "bad");
    finish(false, t("banner_error"), err.text);
    if (err.known) log(err.raw, "dim");
  }
}

async function stop() {
  await api.cancel();
  S.running = false;
  if (S.pick?.side === "recv") closePicker();
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

// ------------------------------------------------------------------ Versions

const RELEASES_API = "https://api.github.com/repos/tetsujin28/lugus-transfer/releases?per_page=15";
const RELEASES_PAGE = "https://github.com/tetsujin28/lugus-transfer/releases";

// "v0.3.0" ou "0.3.0-beta" -> [0, 3, 0] ; null si le texte n'est pas un numéro.
function parseVersion(text) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)/.exec(String(text || "").trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}
// > 0 si a est plus récente que b, < 0 si plus ancienne, 0 si égales.
function compareVersions(a, b) {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

// Interroge GitHub une seule fois. Les pré-versions comptent (la bêta n'en a que),
// les brouillons non. En cas d'échec (hors ligne, limite GitHub), on ne dit rien.
async function checkUpdate() {
  if (!store.get("updateCheck", true)) { S.latest = null; return renderVersion(); }
  try {
    const res = await fetch(RELEASES_API, { headers: { Accept: "application/vnd.github+json" } });
    if (!res.ok) return;
    let best = null;
    for (const r of await res.json()) {
      if (r.draft) continue;
      const v = parseVersion(r.tag_name);
      if (v && (!best || compareVersions(v, best) > 0)) best = v;
    }
    const mine = parseVersion(VERSION);
    S.latest = best && mine && compareVersions(best, mine) > 0 ? best.join(".") : null;
  } catch { S.latest = null; }
  renderVersion();
}

function renderVersion() {
  const b = $("verBadge");
  const news = !!S.latest;
  b.textContent = news ? t("update_badge", { v: S.latest }) : "v" + VERSION;
  b.title = news ? t("update_title") : t("version_title");
  b.classList.toggle("new", news);
}

function settingsBox() {
  return () => {
    const on = store.get("updateCheck", true);
    $("modalBox").innerHTML = `
      <h2>${esc(t("settings"))}</h2>
      <h4>${esc(t("updates_title"))}</h4>
      <label class="opt"><input type="checkbox" id="optUpdate"${on ? " checked" : ""}>
        <span>${esc(t("update_check"))}</span></label>
      <p>${esc(t("update_check_sub"))}</p>
      <div class="acts"><button class="go" id="modalOk">${esc(t("close"))}</button></div>`;
    $("optUpdate").onchange = () => { store.set("updateCheck", $("optUpdate").checked); checkUpdate(); };
    $("modalOk").onclick = closeModal;
  };
}

// ------------------------------------------------------------------ Démarrage

function init() {
  Promise.resolve(window.__TAURI__?.app?.getVersion?.())
    .then((v) => { if (v) VERSION = v; S.modalFn?.(); renderVersion(); return checkUpdate(); })
    .catch(() => checkUpdate());
  if (api.demo) $("chip").insertAdjacentHTML("afterend", '<span class="demo" data-i18n="demo"></span>');

  document.querySelectorAll("#lang button").forEach((b) => (b.onclick = () => setLang(b.dataset.l)));
  $("about").onclick = () => modal(aboutBox(false));
  $("settings").onclick = () => modal(settingsBox());
  $("verBadge").onclick = () => (S.latest ? api.openReleases(RELEASES_PAGE).catch(() => {}) : modal(aboutBox(false)));
  $("modeSend").onclick = () => setMode("send");
  $("modeRecv").onclick = () => setMode("recv");

  $("drop").onclick = async (ev) => {
    if (ev.target.closest("#clearSource")) { ev.stopPropagation(); return clearSource(); }
    const rm = ev.target.closest("[data-rm]");
    if (rm) { ev.stopPropagation(); return removeSource(Number(rm.dataset.rm)); }
    addSources(await api.pickFolder(store.get("lastSource", null)));
  };
  $("pickFolder").onclick = async () => addSources(await api.pickFolder(store.get("lastSource", null)));
  $("pickFiles").onclick = async () => addSources(await api.pickFiles());
  api.onDrop((paths) => { if (S.mode !== "send") setMode("send"); addSources(paths); });

  $("pickDest").onclick = async () => {
    const p = await api.pickDest(S.dest);
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

  initPicker();
  api.onEvent(onEvent);
  applyLang();
  renderVersion();
  renderNight();
  setMode(S.mode);
  if (!store.get("accepted", false)) modal(aboutBox(true));
}

init();

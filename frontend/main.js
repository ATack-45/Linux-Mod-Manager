import { loadModsPage } from "./modding/index.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Maps NexusMods game domain → Steam app_id
const NEXUS_DOMAIN_TO_APP_ID = {
  "cyberpunk2077": "1091500",
};

// ── State ──────────────────────────────────────────────────────────
let autoPaths = [];
let settings = { extra_paths: [], excluded_paths: [], pinned_games: [], nexus_api_key: "" };
let allPaths = [];
let viewMode = "tile"; // "tile" | "list"

// Pending library edits (not yet saved)
let pendingExcluded = new Set();
let pendingExtra = [];
let isDirty = false;

// Games cache — populated after each scan, used by sidebar pins
let gameMap = {}; // app_id -> { game, coverDataUrl }

// Mods page state
let currentModGame = null;

// ── Navigation ─────────────────────────────────────────────────────
function navigateRaw(pageId) {
  document.querySelectorAll(".page").forEach((p) => p.classList.remove("active"));
  document.getElementById(`page-${pageId}`).classList.add("active");
}

function navigate(pageId) {
  document.querySelectorAll(".nav-btn").forEach((b) => b.classList.remove("active"));
  const navBtn = document.querySelector(`[data-page="${pageId}"]`);
  if (navBtn) navBtn.classList.add("active");
  navigateRaw(pageId);
}

// ── Games ──────────────────────────────────────────────────────────
async function loadGames() {
  const status = document.getElementById("games-status");
  const grid = document.getElementById("games-grid");
  const count = document.getElementById("games-count");

  grid.innerHTML = "";
  count.textContent = "";
  status.classList.remove("hidden");
  status.textContent = "Scanning libraries...";

  try {
    const games = await invoke("scan_games", { paths: allPaths });

    if (games.length === 0) {
      status.textContent = allPaths.length === 0
        ? "No Steam libraries found. Add a path in Settings."
        : "No installed games found in the configured libraries.";
      gameMap = {};
      renderPinnedSection();
      return;
    }

    status.classList.add("hidden");
    status.textContent = "";
    count.textContent = `${games.length} game${games.length !== 1 ? "s" : ""}`;

    // Fetch all cover images in parallel before rendering
    const coverDataUrls = await Promise.all(
      games.map((g) =>
        g.cover_path
          ? invoke("read_cover_image", { path: g.cover_path }).catch(() => null)
          : Promise.resolve(null)
      )
    );

    // Rebuild game map for sidebar pin lookups
    gameMap = {};
    games.forEach((game, i) => {
      gameMap[game.app_id] = { game, coverDataUrl: coverDataUrls[i] };
    });

    games.forEach((game, i) => grid.appendChild(makeGameCard(game, coverDataUrls[i])));
    renderPinnedSection();
  } catch (err) {
    status.textContent = `Error scanning games: ${err}`;
  }
}

function makeGameCard(game, coverDataUrl) {
  const card = document.createElement("div");
  card.className = "game-card";
  card.addEventListener("click", () => openModsPage(game, coverDataUrl));

  const libShort = game.library_path.split("/").slice(-2).join("/");

  // Tile cover (tall, hidden in list view)
  const coverEl = document.createElement("div");
  coverEl.className = "game-cover";
  if (coverDataUrl) {
    const img = document.createElement("img");
    img.alt = "";
    img.src = coverDataUrl;
    coverEl.appendChild(img);
  } else {
    coverEl.appendChild(makePlaceholder(game.name));
  }

  // List icon (small square, hidden in tile view)
  const iconEl = document.createElement("div");
  iconEl.className = "game-list-icon";
  if (coverDataUrl) {
    const img = document.createElement("img");
    img.alt = "";
    img.src = coverDataUrl;
    iconEl.appendChild(img);
  } else {
    const ph = document.createElement("div");
    ph.className = "game-list-icon-placeholder";
    ph.textContent = (game.name || "?")[0].toUpperCase();
    iconEl.appendChild(ph);
  }

  const infoEl = document.createElement("div");
  infoEl.className = "game-info";
  infoEl.innerHTML =
    `<div class="game-name" title="${esc(game.name)}">${esc(game.name)}</div>` +
    `<div class="game-library">${esc(libShort)}</div>` +
    `<div class="game-size">${formatSize(game.size_on_disk)}</div>`;

  // Pin button — stops propagation so it doesn't open the mods page
  const pinBtn = document.createElement("button");
  pinBtn.className = "pin-btn";
  pinBtn.title = "Pin to sidebar";
  const isPinned = (settings.pinned_games || []).includes(game.app_id);
  pinBtn.classList.toggle("pinned", isPinned);
  pinBtn.textContent = isPinned ? "★" : "☆";
  pinBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    togglePin(game.app_id, pinBtn);
  });

  card.appendChild(coverEl);
  card.appendChild(iconEl);
  card.appendChild(infoEl);
  card.appendChild(pinBtn);
  return card;
}

function makePlaceholder(name) {
  const letter = (name || "?")[0].toUpperCase();
  const el = document.createElement("div");
  el.className = "game-cover-placeholder";
  const span = document.createElement("span");
  span.className = "game-cover-placeholder-letter";
  span.textContent = letter;
  el.appendChild(span);
  return el;
}

function setViewMode(mode) {
  viewMode = mode;
  const grid = document.getElementById("games-grid");
  grid.classList.toggle("list-view", mode === "list");
  document.getElementById("btn-view-tile").classList.toggle("active", mode === "tile");
  document.getElementById("btn-view-list").classList.toggle("active", mode === "list");
}

// ── Pins ───────────────────────────────────────────────────────────
function togglePin(appId, pinBtnEl) {
  if (!settings.pinned_games) settings.pinned_games = [];
  const idx = settings.pinned_games.indexOf(appId);
  if (idx === -1) {
    settings.pinned_games.push(appId);
  } else {
    settings.pinned_games.splice(idx, 1);
  }

  // Update the card's pin button appearance
  const pinned = settings.pinned_games.includes(appId);
  if (pinBtnEl) {
    pinBtnEl.classList.toggle("pinned", pinned);
    pinBtnEl.textContent = pinned ? "★" : "☆";
  }

  renderPinnedSection();
  invoke("save_settings", { settings }).catch((err) =>
    console.error("Failed to save pin state:", err)
  );
}

function renderPinnedSection() {
  const section = document.getElementById("pinned-section");
  const list = document.getElementById("pinned-list");
  const pins = settings.pinned_games || [];

  // Only show games that are currently installed (in gameMap)
  const activePins = pins.filter((id) => gameMap[id]);

  section.classList.toggle("hidden", activePins.length === 0);
  list.innerHTML = "";

  for (const appId of activePins) {
    const { game, coverDataUrl } = gameMap[appId];
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.className = "pinned-game-btn";
    btn.title = game.name;
    btn.textContent = game.name;
    btn.addEventListener("click", () => openModsPage(game, coverDataUrl));
    li.appendChild(btn);
    list.appendChild(li);
  }
}

// ── Mods page ──────────────────────────────────────────────────────
async function openModsPage(game, coverDataUrl, opts = {}) {
  currentModGame = game;

  // Populate hero cover
  const coverEl = document.getElementById("mods-game-cover");
  coverEl.innerHTML = "";
  coverEl.className = "mods-game-cover";
  if (coverDataUrl) {
    const img = document.createElement("img");
    img.alt = "";
    img.src = coverDataUrl;
    coverEl.appendChild(img);
  } else {
    coverEl.className = "mods-game-cover-placeholder";
    coverEl.textContent = (game.name || "?")[0].toUpperCase();
  }

  document.getElementById("mods-game-name").textContent = game.name;
  document.getElementById("mods-game-path").textContent =
    `${game.library_path}/steamapps/common/${game.install_dir}`;

  const body = document.getElementById("mods-body");
  body.innerHTML = '<div class="mods-loading">Scanning mods...</div>';

  navigateRaw("mods");

  await loadModsPage(game, coverDataUrl, body, opts);
}

function closeModsPage() {
  currentModGame = null;
  navigateRaw("games");
}

// ── Settings — rendering ───────────────────────────────────────────
function pendingPaths() {
  return [
    ...autoPaths.filter((p) => !pendingExcluded.has(p)),
    ...pendingExtra,
  ];
}

function renderLibraryList() {
  const list = document.getElementById("library-list");
  list.innerHTML = "";
  const paths = pendingPaths();

  if (paths.length === 0) {
    const p = document.createElement("p");
    p.className = "library-list-empty";
    p.textContent = "No Steam libraries configured. Add a path with the + button above.";
    list.appendChild(p);
    return;
  }

  paths.forEach((path) => {
    const li = document.createElement("li");
    li.className = "library-item";
    li.innerHTML =
      `<span class="library-item-path" title="${esc(path)}">${esc(path)}</span>` +
      `<button class="library-item-remove" title="Remove">&times;</button>`;
    li.querySelector("button").addEventListener("click", () => removePath(path));
    list.appendChild(li);
  });
}

// ── Settings — editing ─────────────────────────────────────────────
function removePath(path) {
  if (autoPaths.includes(path)) {
    pendingExcluded.add(path);
  } else {
    pendingExtra = pendingExtra.filter((p) => p !== path);
  }
  markDirty();
  renderLibraryList();
}

function addPath(path) {
  path = path.trim();
  if (!path) return;
  if (pendingPaths().includes(path)) return;

  pendingExtra.push(path);
  markDirty();
  renderLibraryList();
}

function markDirty() {
  isDirty = true;
  const btn = document.getElementById("btn-save-settings");
  btn.disabled = false;
  document.getElementById("save-status").textContent = "Unsaved changes";
}

async function saveSettings() {
  const btn = document.getElementById("btn-save-settings");
  const status = document.getElementById("save-status");
  btn.disabled = true;
  status.textContent = "Saving...";

  settings.excluded_paths = [...pendingExcluded];
  settings.extra_paths = pendingExtra;

  try {
    await invoke("save_settings", { settings });
    isDirty = false;
    allPaths = pendingPaths();
    status.textContent = "Saved";
    setTimeout(() => { status.textContent = ""; }, 2000);
    loadGames();
  } catch (err) {
    btn.disabled = false;
    status.textContent = `Error: ${err}`;
  }
}

// ── Add library input toggle ───────────────────────────────────────
function showAddRow() {
  document.getElementById("add-library-row").classList.remove("hidden");
  document.getElementById("add-library-input").focus();
}

function hideAddRow() {
  document.getElementById("add-library-row").classList.add("hidden");
  document.getElementById("add-library-input").value = "";
}

function confirmAdd() {
  const input = document.getElementById("add-library-input");
  addPath(input.value);
  hideAddRow();
}

// ── Init ───────────────────────────────────────────────────────────
async function init() {
  try {
    [autoPaths, settings] = await Promise.all([
      invoke("discover_steam_libraries"),
      invoke("load_settings"),
    ]);
  } catch (err) {
    console.error("Failed to load initial data:", err);
    autoPaths = [];
    settings = { extra_paths: [], excluded_paths: [], pinned_games: [] };
  }

  // Migrate old settings format if needed
  if (!settings.extra_paths) settings.extra_paths = settings.custom_library_paths || [];
  if (!settings.excluded_paths) settings.excluded_paths = [];
  if (!settings.pinned_games) settings.pinned_games = [];
  if (settings.nexus_api_key === undefined) settings.nexus_api_key = "";

  const nexusInput = document.getElementById("nexus-api-input");
  if (nexusInput) nexusInput.value = settings.nexus_api_key;

  pendingExcluded = new Set(settings.excluded_paths);
  pendingExtra = [...settings.extra_paths];
  allPaths = pendingPaths();

  renderLibraryList();
  await loadGames();

  // Pick up any NXM link that arrived before the webview was ready
  const pendingNxm = await invoke("get_pending_nxm_url");
  if (pendingNxm) handleNxmUrl(pendingNxm);
}

// ── Helpers ────────────────────────────────────────────────────────
function formatSize(bytes) {
  if (bytes >= 1e9) return (bytes / 1e9).toFixed(1) + " GB";
  if (bytes >= 1e6) return (bytes / 1e6).toFixed(0) + " MB";
  return bytes + " B";
}

function esc(str) {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ── Event listeners ────────────────────────────────────────────────
document.querySelectorAll(".nav-btn").forEach((btn) => {
  btn.addEventListener("click", () => navigate(btn.dataset.page));
});

document.getElementById("btn-refresh-games").addEventListener("click", loadGames);
document.getElementById("btn-view-tile").addEventListener("click", () => setViewMode("tile"));
document.getElementById("btn-view-list").addEventListener("click", () => setViewMode("list"));

document.getElementById("btn-show-add-library").addEventListener("click", showAddRow);
document.getElementById("btn-cancel-add").addEventListener("click", hideAddRow);
document.getElementById("btn-confirm-add").addEventListener("click", confirmAdd);

document.getElementById("add-library-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") confirmAdd();
  if (e.key === "Escape") hideAddRow();
});

document.getElementById("btn-save-settings").addEventListener("click", saveSettings);
document.getElementById("btn-back-to-games").addEventListener("click", closeModsPage);

document.getElementById("nexus-api-input").addEventListener("input", (e) => {
  settings.nexus_api_key = e.target.value.trim();
  markDirty();
});

document.getElementById("btn-nexus-api-show").addEventListener("click", (e) => {
  const input = document.getElementById("nexus-api-input");
  const show = input.type === "password";
  input.type = show ? "text" : "password";
  e.target.textContent = show ? "Hide" : "Show";
});

// ── NXM deep-link handler ──────────────────────────────────────────
function handleNxmUrl(nxmUrl) {
  const match = String(nxmUrl).match(/^nxm:\/\/([^/]+)/i);
  if (!match) return;
  const domain = match[1].toLowerCase();
  const appId = NEXUS_DOMAIN_TO_APP_ID[domain];
  if (!appId) return;
  const entry = gameMap[appId];
  if (!entry) return;
  openModsPage(entry.game, entry.coverDataUrl, { nxmUrl });
}

// Live event: clear pending so the init-time check doesn't double-process
listen("nxm-link", ({ payload }) => {
  invoke("clear_pending_nxm_url").catch(() => {});
  handleNxmUrl(payload);
});

init();

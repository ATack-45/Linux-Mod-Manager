const { invoke, convertFileSrc } = window.__TAURI__.core;

// ── State ──────────────────────────────────────────────────────────
let autoPaths = [];
let settings = { extra_paths: [], excluded_paths: [] };
let allPaths = [];
let viewMode = "tile"; // "tile" | "list"

// Pending edits (not yet saved)
let pendingExcluded = new Set();
let pendingExtra = [];
let isDirty = false;

// ── Navigation ─────────────────────────────────────────────────────
function navigate(pageId) {
  document.querySelectorAll(".nav-btn").forEach((b) => b.classList.remove("active"));
  document.querySelectorAll(".page").forEach((p) => p.classList.remove("active"));
  document.querySelector(`[data-page="${pageId}"]`).classList.add("active");
  document.getElementById(`page-${pageId}`).classList.add("active");
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

    games.forEach((game, i) => grid.appendChild(makeGameCard(game, coverDataUrls[i])));
  } catch (err) {
    status.textContent = `Error scanning games: ${err}`;
  }
}

function makeGameCard(game, coverDataUrl) {
  const card = document.createElement("div");
  card.className = "game-card";

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

  card.appendChild(coverEl);
  card.appendChild(iconEl);
  card.appendChild(infoEl);
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
    settings = { extra_paths: [], excluded_paths: [] };
  }

  // Migrate old settings format if needed
  if (!settings.extra_paths) settings.extra_paths = settings.custom_library_paths || [];
  if (!settings.excluded_paths) settings.excluded_paths = [];

  pendingExcluded = new Set(settings.excluded_paths);
  pendingExtra = [...settings.extra_paths];
  allPaths = pendingPaths();

  renderLibraryList();
  loadGames();
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

init();

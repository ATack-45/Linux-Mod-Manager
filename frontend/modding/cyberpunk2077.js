const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ── Helpers ───────────────────────────────────────────────────────────

function esc(str) {
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function formatKb(kb) {
  if (kb >= 1024) return (kb / 1024).toFixed(1) + " MB";
  return kb + " KB";
}

// ── Installed mods list ───────────────────────────────────────────────

// displayMod: { id, name, version, enabled, managed, components[] }
function makeModRow(displayMod, game) {
  const row = document.createElement("div");
  row.className = "mod-row";

  const nameEl = document.createElement("span");
  nameEl.className = "mod-row-name";
  nameEl.title = displayMod.name;
  nameEl.textContent = displayMod.name;

  const toggleBtn = document.createElement("button");
  toggleBtn.className = "mod-toggle-btn";

  let currentEnabled = displayMod.enabled;

  function syncToggle() {
    toggleBtn.dataset.enabled = String(currentEnabled);
    toggleBtn.textContent = currentEnabled ? "Enabled" : "Disabled";
  }
  syncToggle();

  toggleBtn.addEventListener("click", async () => {
    const newEnabled = !currentEnabled;
    toggleBtn.disabled = true;
    row.classList.remove("error");
    try {
      await invoke("toggle_display_mod", {
        components: displayMod.components,
        enabled: newEnabled,
      });
      currentEnabled = newEnabled;
      syncToggle();
    } catch (err) {
      row.classList.add("error");
      row.title = String(err);
    } finally {
      toggleBtn.disabled = false;
    }
  });

  // Uninstall button — shows inline confirmation before deleting
  const uninstallBtn = document.createElement("button");
  uninstallBtn.className = "mod-uninstall-btn";
  uninstallBtn.title = "Uninstall mod";
  uninstallBtn.textContent = "Uninstall";

  uninstallBtn.addEventListener("click", () => {
    nameEl.textContent = `Remove ${displayMod.name} permanently?`;
    toggleBtn.style.display = "none";
    uninstallBtn.style.display = "none";

    const cancelBtn = document.createElement("button");
    cancelBtn.className = "ghost-btn mod-confirm-btn";
    cancelBtn.textContent = "Cancel";
    cancelBtn.addEventListener("click", () => {
      nameEl.textContent = label;
      toggleBtn.style.display = "";
      uninstallBtn.style.display = "";
      row.removeChild(cancelBtn);
      row.removeChild(confirmBtn);
    });

    const confirmBtn = document.createElement("button");
    confirmBtn.className = "mod-uninstall-btn mod-confirm-btn";
    confirmBtn.textContent = "Remove";
    confirmBtn.addEventListener("click", async () => {
      confirmBtn.disabled = true;
      cancelBtn.disabled = true;
      try {
        await invoke("uninstall_display_mod", {
          appId: game.app_id,
          modId: displayMod.id,
          components: displayMod.components,
        });
        row.remove();
      } catch (err) {
        nameEl.textContent = `Failed: ${err}`;
        nameEl.style.color = "var(--danger)";
        cancelBtn.textContent = "Close";
        cancelBtn.disabled = false;
        confirmBtn.style.display = "none";
      }
    });

    row.appendChild(cancelBtn);
    row.appendChild(confirmBtn);
  });

  row.appendChild(nameEl);
  row.appendChild(toggleBtn);
  row.appendChild(uninstallBtn);
  return row;
}

async function renderModsList(game, container) {
  container.innerHTML = '<div class="mods-loading">Scanning mods...</div>';

  const installPath = `${game.library_path}/steamapps/common/${game.install_dir}`;
  let mods;
  try {
    mods = await invoke("list_display_mods", {
      appId: game.app_id,
      installDir: installPath,
    });
  } catch (err) {
    container.innerHTML = `<div class="mods-empty-state">Failed to scan mods: ${esc(String(err))}</div>`;
    return;
  }

  container.innerHTML = "";

  if (mods.length === 0) {
    container.innerHTML = '<div class="mods-empty-state">No mods installed yet.</div>';
    return;
  }

  for (const displayMod of mods) {
    container.appendChild(makeModRow(displayMod, game));
  }
}

// ── NexusMods install panel ───────────────────────────────────────────

// Extract file_id from an NXM link, null if not present (plain URL)
function parseNxmFileId(url) {
  const match = String(url).match(/^nxm:\/\/[^/]+\/mods\/\d+\/files\/(\d+)/i);
  return match ? Number(match[1]) : null;
}

function renderInstallPanel(game, modsListContainer) {
  const panel = document.createElement("div");
  panel.className = "nexus-install-panel";

  showInputState(panel, game, modsListContainer);
  return panel;
}

function showInputState(panel, game, modsListContainer) {
  panel.innerHTML = `
    <div class="nexus-panel-title">Install from NexusMods</div>
    <div class="nexus-input-row">
      <input
        class="nexus-url-input"
        type="text"
        placeholder="Paste nexusmods.com URL or NXM link..."
        spellcheck="false"
        autocomplete="off"
      />
      <button class="action-btn nexus-lookup-btn">Look up</button>
    </div>
    <div class="nexus-panel-error hidden"></div>
  `;

  const input = panel.querySelector(".nexus-url-input");
  const btn = panel.querySelector(".nexus-lookup-btn");
  const errEl = panel.querySelector(".nexus-panel-error");

  async function doLookup() {
    const url = input.value.trim();
    if (!url) return;
    btn.disabled = true;
    btn.textContent = "Looking up...";
    errEl.classList.add("hidden");

    const nxmFileId = parseNxmFileId(url);

    try {
      const apiKey = await getApiKey();
      const [info, files] = await invoke("nexus_lookup", { apiKey, input: url });

      if (nxmFileId !== null) {
        // NXM link carries the exact file — skip the picker entirely
        const file = files.find((f) => f.file_id === nxmFileId);
        const fileName = file ? file.name : info.name;
        showDownloadState(panel, game, modsListContainer, url, nxmFileId, fileName, info, files);
      } else {
        // Plain URL — show the file picker
        showFilePickerState(panel, game, modsListContainer, url, info, files);
      }
    } catch (err) {
      errEl.textContent = String(err);
      errEl.classList.remove("hidden");
      btn.disabled = false;
      btn.textContent = "Look up";
    }
  }

  btn.addEventListener("click", doLookup);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") doLookup();
  });
}

function showFilePickerState(panel, game, modsListContainer, originalInput, info, files) {
  const fileRows = files.map((f) => `
    <div class="nexus-file-row">
      <div class="nexus-file-info">
        <span class="nexus-file-name">${esc(f.name)}</span>
        <span class="nexus-file-meta">${esc(f.version)} &middot; ${formatKb(f.size_kb)} &middot; ${esc(f.category)}</span>
      </div>
      <button class="action-btn nexus-install-btn" data-file-id="${f.file_id}" data-file-name="${esc(f.name)}">
        Install
      </button>
    </div>
  `).join("");

  panel.innerHTML = `
    <div class="nexus-panel-title">Install from NexusMods</div>
    <div class="nexus-mod-preview">
      <div class="nexus-mod-info">
        <div class="nexus-mod-name">${esc(info.name)}</div>
        <div class="nexus-mod-summary">${esc(info.summary)}</div>
      </div>
    </div>
    <div class="nexus-files-list">${fileRows}</div>
    <div class="nexus-panel-error hidden"></div>
    <button class="ghost-btn nexus-back-btn">&#8592; Different mod</button>
  `;

  panel.querySelector(".nexus-back-btn").addEventListener("click", () => {
    showInputState(panel, game, modsListContainer);
  });

  panel.querySelectorAll(".nexus-install-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const fileId = Number(btn.dataset.fileId);
      const fileName = btn.dataset.fileName;
      showDownloadState(panel, game, modsListContainer, originalInput, fileId, fileName, info, files);
    });
  });
}

function showDownloadState(panel, game, modsListContainer, originalInput, fileId, fileName, info, files) {
  panel.innerHTML = `
    <div class="nexus-panel-title">Install from NexusMods</div>
    <div class="nexus-downloading">
      <div class="nexus-downloading-label">Downloading <strong>${esc(fileName)}</strong>...</div>
      <div class="nexus-progress-bar"><div class="nexus-progress-fill" style="width:0%"></div></div>
      <div class="nexus-progress-text">0%</div>
    </div>
    <div class="nexus-panel-error hidden"></div>
  `;

  const fill = panel.querySelector(".nexus-progress-fill");
  const pctText = panel.querySelector(".nexus-progress-text");

  let unlisten;
  listen("download-progress", ({ payload }) => {
    fill.style.width = `${payload.pct}%`;
    pctText.textContent = `${payload.pct}%  (${payload.downloaded_kb} / ${payload.total_kb} KB)`;
  }).then((fn) => { unlisten = fn; });

  const installDir = `${game.library_path}/steamapps/common/${game.install_dir}`;
  const file = files ? files.find((f) => f.file_id === fileId) : null;

  getApiKey().then((apiKey) =>
    invoke("nexus_install", {
      apiKey,
      input: originalInput,
      fileId,
      installDir,
      appId: game.app_id,
      modName: info.name,
      modVersion: file ? file.version : (info.version || ""),
      nexusModId: info.mod_id,
    })
  ).then(async (installedPaths) => {
    if (unlisten) unlisten();
    showSuccessState(panel, game, modsListContainer, installedPaths);
    await renderModsList(game, modsListContainer);
  }).catch((err) => {
    if (unlisten) unlisten();
    const errEl = panel.querySelector(".nexus-panel-error");
    errEl.textContent = String(err);
    errEl.classList.remove("hidden");
    // Let user go back to file picker
    const backBtn = document.createElement("button");
    backBtn.className = "ghost-btn";
    backBtn.style.marginTop = "10px";
    backBtn.textContent = "Back";
    backBtn.addEventListener("click", () =>
      showFilePickerState(panel, game, modsListContainer, originalInput, info, files)
    );
    panel.appendChild(backBtn);
  });
}

function showSuccessState(panel, game, modsListContainer, installedPaths) {
  panel.innerHTML = `
    <div class="nexus-panel-title">Install from NexusMods</div>
    <div class="nexus-success">
      Installed ${installedPaths.length} file${installedPaths.length !== 1 ? "s" : ""} successfully.
    </div>
    <button class="ghost-btn nexus-reset-btn" style="margin-top: 10px;">Install another mod</button>
  `;
  panel.querySelector(".nexus-reset-btn").addEventListener("click", () => {
    showInputState(panel, game, modsListContainer);
  });
}

async function getApiKey() {
  const s = await invoke("load_settings");
  const key = s.nexus_api_key || "";
  if (!key) {
    throw new Error("No NexusMods API key configured. Add yours in Settings.");
  }
  return key;
}

// ── Entry point ───────────────────────────────────────────────────────

export async function render(game, _coverDataUrl, container, opts = {}) {
  container.innerHTML = "";

  // Install panel (always shown at top)
  const modsListContainer = document.createElement("div");
  modsListContainer.className = "mods-installed-section";

  const panel = renderInstallPanel(game, modsListContainer);
  container.appendChild(panel);

  // If we arrived here from an NXM deep-link, auto-fill and look up
  if (opts.nxmUrl) {
    const input = panel.querySelector(".nexus-url-input");
    const btn = panel.querySelector(".nexus-lookup-btn");
    if (input && btn) {
      input.value = opts.nxmUrl;
      btn.click();
    }
  }

  const divider = document.createElement("div");
  divider.className = "nexus-section-divider";
  divider.textContent = "Installed Mods";
  container.appendChild(divider);

  container.appendChild(modsListContainer);

  await renderModsList(game, modsListContainer);
}

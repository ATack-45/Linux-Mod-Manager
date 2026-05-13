// ── Mod module registry ──────────────────────────────────────────────
// To add a new game: add its app_id here with a dynamic import thunk.

const MOD_MODULES = {
  "1091500": () => import("./cyberpunk2077.js"),
};

// ── Dispatcher ───────────────────────────────────────────────────────

export async function loadModsPage(game, coverDataUrl, container, opts = {}) {
  const loader = MOD_MODULES[game.app_id];
  const module = loader
    ? await loader()
    : await import("./default.js");
  await module.render(game, coverDataUrl, container, opts);
}

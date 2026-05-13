function esc(str) {
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export async function render(game, _coverDataUrl, container) {
  container.innerHTML = `
    <div class="mods-unsupported">
      <div class="mods-unsupported-title">Mod support not yet available</div>
      <div class="mods-unsupported-desc">
        Mod management for <strong>${esc(game.name)}</strong> has not been implemented yet.
      </div>
    </div>
  `;
}

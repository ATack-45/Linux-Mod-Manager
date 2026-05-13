#!/bin/bash
set -e

APP_NAME="linux-mod-manager"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Linux Mod Manager -- Setup"
echo "==========================="

# ── Install Rust ───────────────────────────────────────────────────
install_rust() {
    if command -v cargo &>/dev/null; then
        echo "[ok] Rust already installed ($(rustc --version))"
        return
    fi
    echo "[..] Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    source "$HOME/.cargo/env"

    # Add cargo to PATH permanently in the user's shell profile
    for profile in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
        if [ -f "$profile" ] && ! grep -q 'cargo/env' "$profile"; then
            echo 'source "$HOME/.cargo/env"' >> "$profile"
        fi
    done
    echo "[ok] Rust installed"
}

install_rust

export PATH="$HOME/.cargo/bin:$PATH"

# ── Install WebKitGTK system dependency ───────────────────────────
install_webkit() {
    if pkg-config --exists webkit2gtk-4.1 2>/dev/null || pkg-config --exists webkit2gtk-4.0 2>/dev/null; then
        echo "[ok] WebKitGTK already installed"
        return
    fi

    echo "[..] Installing WebKitGTK..."

    if command -v apt-get &>/dev/null; then
        sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libgtk-3-dev \
            libayatana-appindicator3-dev \
            librsvg2-dev \
            patchelf
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y \
            webkit2gtk4.1-devel \
            gtk3-devel \
            libappindicator-gtk3-devel \
            librsvg2-devel
    elif command -v pacman &>/dev/null; then
        sudo pacman -S --noconfirm \
            webkit2gtk-4.1 \
            gtk3 \
            libappindicator-gtk3 \
            librsvg
    elif command -v zypper &>/dev/null; then
        sudo zypper install -y \
            webkit2gtk3-devel \
            gtk3-devel \
            librsvg-devel
    else
        echo "[error] Could not detect package manager. Install WebKitGTK manually."
        echo "  Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev"
        echo "  Fedora:        sudo dnf install webkit2gtk4.1-devel gtk3-devel"
        echo "  Arch:          sudo pacman -S webkit2gtk-4.1 gtk3"
        exit 1
    fi

    echo "[ok] WebKitGTK installed"
}

install_webkit

# ── Install Tauri CLI ─────────────────────────────────────────────
install_tauri_cli() {
    if cargo tauri --version &>/dev/null 2>&1; then
        echo "[ok] tauri-cli already installed"
        return
    fi
    echo "[..] Installing tauri-cli (this may take a few minutes)..."
    cargo install tauri-cli --version "^2" --locked
    echo "[ok] tauri-cli installed"
}

install_tauri_cli

# ── Launch in dev mode ────────────────────────────────────────────
echo ""
echo "[..] Building and launching Linux Mod Manager..."
echo "    (first build may take several minutes while Rust compiles dependencies)"
echo ""

cd "$SCRIPT_DIR"
cargo tauri dev

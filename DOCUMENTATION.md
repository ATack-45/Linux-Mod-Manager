# Adding Game Support Guide

To add support for a new game, follow these steps:

## 1. Frontend (JavaScript)
Create a new JavaScript file in `frontend/modding/` with the pattern `GAME_NAME.js`. Example:
```javascript
// frontend/modding/newgame.js
export function getModdingConfig() {
  return {
    id: 'newgame',
    name: 'New Game',
    icon: '/modding/icons/newgame.png',
    moddingEnabled: true,
    // ... other game-specific config
  };
}

export function getSupportedGames() {
  return [
    {
      id: 'newgame',
      name: 'New Game',
      icon: '/modding/icons/newgame.png',
      moddingEnabled: true,
      // ... other game-specific config
    }
  ];
}
```

## 2. Backend (Rust)
Create a new Rust module in `src-tauri/src/modding/`:
```rust
// src-tauri/src/modding/newgame.rs
pub mod newgame {
  use tauri::State;
  
  pub fn init(state: &mut State) {
    state.register_game(
      "newgame",
      "New Game",
      vec!["/path/to/game/exe"],
      Some("/modding/icons/newgame.png"),
    );
  }
}
```

## 3. Register Game
Update `src-tauri/src/modding/mod.rs` to include your new module:
```rust
// src-tauri/src/modding/mod.rs
pub mod newgame;

pub fn init(state: &mut State) {
  newgame::init(state);
  // ... other game initializers
}
```

## 4. (Optional) Configuration
If needed, add game-specific settings to `src-tauri/capabilities/default.json`:
```json
{
  "newgame": {
    "supported": true,
    "modding": {
      "enabled": true,
      "icon": "modding/icons/newgame.png"
    }
  }
}
```

## 5. UI Integration
Update `frontend/index.html` to include game detection:
```html
<!-- frontend/index.html -->
<div id="game-list"></div>
<script>
  window.addEventListener('DOMContentLoaded', () => {
    fetch('/modding/getSupportedGames').then(response => {
      response.json().then(games => {
        const list = document.getElementById('game-list');
        games.forEach(game => {
          const item = document.createElement('div');
          item.textContent = game.name;
          list.appendChild(item);
        });
      });
    });
  });
</script>
```

## 6. (Optional) Icon
Create an icon file at `frontend/modding/icons/newgame.png`

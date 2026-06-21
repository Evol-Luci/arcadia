# Custom display name for games — design

**Date:** 2026-06-21
**Status:** Approved (pending implementation)

## Problem

Some library entries display a coded internal name (e.g. `SLU1654`) instead of a
human title. This happens when the scanner's `derive_title` cannot find good
metadata and falls back to the ROM/disc filename. There is currently no way for
the user to correct the displayed name.

## Goal

Let the user set a custom display name for any game. The custom name:

- replaces the displayed title everywhere the game is rendered;
- also drives **sort order and search**, so a renamed game sorts and matches
  under its new name;
- **survives library rescans** (the scanner must never clobber it);
- is **reversible** — clearing the override falls back to the scanned title.

## Non-goals

- Bulk/batch renaming.
- Editing other metadata (genre, developer, etc.).
- Renaming from the library grid context menu (detail page only, for now).

## Data model

The scanned `title` remains the derived fallback; a new nullable column holds the
user override.

### Migration `core/dreamvault/migrations/0008_custom_title.sql`

```sql
ALTER TABLE games ADD COLUMN custom_title TEXT;
```

`NULL` (or empty) means "no override — use the scanned `title`".

### `Game` struct (`core/dreamvault/src/models.rs`)

Add:

```rust
pub custom_title: Option<String>,
```

Both `list_games` and `get_game` use `SELECT *`, so the field hydrates with no
query changes.

## Rescan safety

`insert_game` (`core/dreamvault/src/library.rs`) upserts on
`(profile_id, rom_path)` and currently overwrites `title` and `sort_title` from
the freshly derived values on every rescan. Update the `ON CONFLICT ... DO
UPDATE` clause so a custom name's sort key is preserved:

```sql
ON CONFLICT(profile_id, rom_path) DO UPDATE SET
    title      = excluded.title,
    sort_title = CASE
                   WHEN custom_title IS NOT NULL AND custom_title <> ''
                   THEN games.sort_title
                   ELSE excluded.sort_title
                 END,
    platform   = excluded.platform,
    file_size  = excluded.file_size
```

- `title` keeps refreshing — it is the fallback / "reset to original" target.
- `sort_title` is only refreshed from the scan when there is **no** override;
  when an override exists, the custom-derived sort key set at rename time is kept.
- `custom_title` itself is never written by the scanner.

## Engine API

New method on the library engine (`core/dreamvault/src/library.rs`):

```rust
pub async fn set_custom_title(&self, game_id: &str, title: Option<&str>) -> Result<()>
```

Behavior:

- **Set** (`Some(name)` after trimming, non-empty):
  `UPDATE games SET custom_title = ?, sort_title = ? WHERE id = ?`
  where `sort_title = sort_key(name)`. Sort and search reflect the new name
  immediately (both already operate on `sort_title`).
- **Clear** (`None`, or a name that trims to empty): read the row's derived
  `title`, then
  `UPDATE games SET custom_title = NULL, sort_title = sort_key(title) WHERE id = ?`
  so sorting reverts to the scanned title's key.

`sort_key` is the existing helper used elsewhere in `library.rs`.

## Tauri command

`core/dreamvault` is wrapped by `desktop/src-tauri/src/commands.rs`. Add,
mirroring `set_favorite`:

```rust
#[tauri::command]
pub async fn set_custom_title(
    state: State<'_, AppState>,
    game_id: String,
    title: Option<String>,
) -> CmdResult<()> {
    map(state.engine.set_custom_title(&game_id, title.as_deref()).await)
}
```

Register it in the Tauri `invoke_handler` (`desktop/src-tauri/src/main.rs`).

## Frontend

### Types (`desktop/src/api/types.ts`)

Add to the `Game` type:

```ts
custom_title: string | null;
```

### Command wrapper (`desktop/src/api/commands.ts`)

```ts
setCustomTitle: (gameId: string, title: string | null) =>
  invoke<void>("set_custom_title", { gameId, title }),
```

### Display helper

A single helper, e.g. `displayTitle(g: Game) => g.custom_title || g.title`,
applied at **every** site that currently renders a game's `title`:

- `desktop/src/views/GameDetail.tsx` — header `<h1>` (line ~282) and the cover
  `alt`/placeholder uses.
- Library grid card component (whatever renders `g.title` per card).
- `desktop/src/views/Favorites.tsx`.
- `desktop/src/components/Sidebar.tsx` if it shows game titles.
- The achievements `title` prop passed from GameDetail.

Implementation note: grep for `.title` on game objects to enumerate sites; the
helper keeps the change mechanical and avoids missing a spot.

### Rename UI (GameDetail header)

- A small edit/pencil control beside the title.
- Activating it swaps the `<h1>` for a text input prefilled with the current
  display name (custom if set, else derived).
- **Save** calls `commands.setCustomTitle(id, value)` and refreshes the game so
  the new name propagates (optimistic update or refetch).
- A **Reset to original** affordance calls `setCustomTitle(id, null)`, reverting
  to `g.title`. Only meaningful when an override exists.
- Empty input on save is treated as a clear.

## Rejected alternatives

1. **Edit `title` in place.** A rescan's upsert overwrites `title`, so the user's
   name would silently vanish on the next scan. Rejected.
2. **Resolve the effective name in SQL** (`COALESCE(custom_title, title) AS
   title`). Avoids frontend display changes but requires replacing `SELECT *`
   with explicit column projections across multiple queries and changes the
   meaning of `Game.title`. More invasive; rejected in favor of the override
   column + a small frontend helper.

## Testing

- **Rust unit/integration:** set a custom title then run a rescan over the same
  ROM — assert `custom_title` and the custom `sort_title` persist while `title`
  refreshes. Clear the override — assert `sort_title` reverts to
  `sort_key(title)`. Assert `list_games` sort/search honor the custom name.
- **Manual:** rename a coded entry (e.g. `SLU1654`) in GameDetail; confirm the
  new name shows on the detail page, library grid, and Favorites, that it sorts
  and searches under the new name, and that "Reset to original" restores the
  scanned title. (Build + install to `/usr/bin/arcadia` per project install
  notes.)

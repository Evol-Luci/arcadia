# Custom Game Display Name — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users override the displayed title of a library game (e.g. rename a coded `SLU1654` entry), with the override surviving rescans and driving sort/search.

**Architecture:** Add a nullable `custom_title` column to `games`. The scanned `title` stays as the derived fallback; `custom_title` wins for display. Sorting/search ride on the existing `sort_title` column, which `set_custom_title` updates to the custom name's key and the rescan upsert preserves when an override exists. The `Game` struct carries `custom_title` so the frontend resolves `custom_title || title` for display (it needs the original for "reset to original"); the read-only Stats query resolves the effective name in SQL.

**Tech Stack:** Rust (`core/dreamvault`, sqlx + SQLite), Tauri (`desktop/src-tauri`), React + TypeScript + react-query (`desktop`).

## Global Constraints

- SQLite migrations live in `core/dreamvault/migrations/` and run automatically via `sqlx::migrate!("./migrations")` (`core/dreamvault/src/db.rs:49`). The next migration number is `0008`.
- Rust tests use `#[tokio::test]` with `db::connect_in_memory()` + `Engine::with_pool(pool)` + `engine.ensure_default_profile()`. The pool is reachable as `engine.pool` inside the `library.rs` test module.
- The scanner-derived sort key is produced by the existing private `fn sort_key(title: &str) -> String` (`core/dreamvault/src/library.rs:979`). Reuse it; do not reimplement normalization.
- Frontend has **no test runner**. Verify frontend changes with `cd desktop && npm run build` (runs `tsc --noEmit && vite build`).
- Tauri commands must be both defined in `desktop/src-tauri/src/commands.rs` and registered in the `generate_handler!` macro in `desktop/src-tauri/src/main.rs` (around line 104).
- Build the Rust workspace with `cargo build` / test with `cargo test -p dreamvault`.
- Install for manual testing per project convention: build `target/release/dreamshell` with `--features custom-protocol`, then atomic-install to `/usr/bin/arcadia` (see project memory; the binary is `dreamshell`, installed as `arcadia`).

---

### Task 1: Add `custom_title` column and struct field

**Files:**
- Create: `core/dreamvault/migrations/0008_custom_title.sql`
- Modify: `core/dreamvault/src/models.rs:82-107` (the `Game` struct)
- Test: `core/dreamvault/src/library.rs` (the existing `#[cfg(test)] mod tests` block at the bottom)

**Interfaces:**
- Consumes: nothing.
- Produces: `Game.custom_title: Option<String>`; the `games.custom_title` SQLite column (nullable `TEXT`, default NULL).

- [ ] **Step 1: Write the failing test**

Add this test inside the `mod tests` block in `core/dreamvault/src/library.rs` (alongside the existing tests):

```rust
#[tokio::test]
async fn new_games_have_no_custom_title() {
    let pool = db::connect_in_memory().await.unwrap();
    let engine = Engine::with_pool(pool).await.unwrap();
    let profile = engine.ensure_default_profile().await.unwrap();

    sqlx::query(
        "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, added_at)
         VALUES ('g1', ?, 'Mario', 'mario', 'snes', '/roms/snes/Mario.sfc', '2026-01-01T00:00:00Z')",
    )
    .bind(&profile.id)
    .execute(&engine.pool)
    .await
    .unwrap();

    let game = engine.get_game("g1").await.unwrap().unwrap();
    assert_eq!(game.custom_title, None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p dreamvault new_games_have_no_custom_title`
Expected: FAIL — compile error, `Game` has no field `custom_title` (and/or the migration/column does not exist).

- [ ] **Step 3: Create the migration**

Create `core/dreamvault/migrations/0008_custom_title.sql`:

```sql
-- User-supplied display name override for a game. NULL/empty means "use the
-- scanned `title`". Survives rescans (see insert_game's ON CONFLICT guard).
ALTER TABLE games ADD COLUMN custom_title TEXT;
```

- [ ] **Step 4: Add the struct field**

In `core/dreamvault/src/models.rs`, add the field to the `Game` struct. Place it directly after `pub title: String,` (line 86) so column order is unambiguous:

```rust
    pub title: String,
    /// User override for the displayed name. NULL when unset; display falls
    /// back to `title`. Set/cleared via `Engine::set_custom_title`.
    pub custom_title: Option<String>,
    pub sort_title: String,
```

(`list_games` and `get_game` use `SELECT *`, so `FromRow` hydrates the new column with no query change. Column order in `SELECT *` follows the table definition; `ALTER TABLE ADD COLUMN` appends `custom_title` at the end, and sqlx `FromRow` matches by name, not position, so placement in the struct is for readability only.)

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p dreamvault new_games_have_no_custom_title`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add core/dreamvault/migrations/0008_custom_title.sql core/dreamvault/src/models.rs core/dreamvault/src/library.rs
git commit -m "feat(library): add custom_title column and Game field"
```

---

### Task 2: `set_custom_title` engine method + rescan-survival guard

**Files:**
- Modify: `core/dreamvault/src/library.rs` — the `insert_game` upsert (`ON CONFLICT` clause around lines 417-421) and a new public method near `set_favorite` (around line 573)
- Test: `core/dreamvault/src/library.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Game.custom_title` (Task 1); `fn sort_key` (`library.rs:979`).
- Produces:
  ```rust
  pub async fn set_custom_title(&self, game_id: &str, title: Option<&str>) -> Result<()>
  ```
  Semantics: a `Some` value that trims to non-empty sets `custom_title` to the **trimmed** value and `sort_title = sort_key(trimmed)`. `None`, or a value that trims to empty, clears the override (`custom_title = NULL`) and resets `sort_title = sort_key(title)` from the row's derived `title`.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `core/dreamvault/src/library.rs`. The first test exercises set/clear; the second proves the override survives a re-`insert_game`.

```rust
#[tokio::test]
async fn set_and_clear_custom_title_updates_sort_key() {
    let pool = db::connect_in_memory().await.unwrap();
    let engine = Engine::with_pool(pool).await.unwrap();
    let profile = engine.ensure_default_profile().await.unwrap();

    sqlx::query(
        "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, added_at)
         VALUES ('g1', ?, 'SLU1654', 'slu1654', 'ps2', '/roms/ps2/SLU1654.iso', '2026-01-01T00:00:00Z')",
    )
    .bind(&profile.id)
    .execute(&engine.pool)
    .await
    .unwrap();

    // Set a custom name (with surrounding whitespace to prove trimming).
    engine
        .set_custom_title("g1", Some("  The Final Fantasy X  "))
        .await
        .unwrap();
    let row: (Option<String>, String, String) =
        sqlx::query_as("SELECT custom_title, sort_title, title FROM games WHERE id = 'g1'")
            .fetch_one(&engine.pool)
            .await
            .unwrap();
    assert_eq!(row.0.as_deref(), Some("The Final Fantasy X"));
    // sort_key lowercases and strips a leading "the ".
    assert_eq!(row.1, "final fantasy x");
    assert_eq!(row.2, "SLU1654"); // derived title untouched

    // Clear it -> sort_title reverts to the derived title's key.
    engine.set_custom_title("g1", None).await.unwrap();
    let row: (Option<String>, String) =
        sqlx::query_as("SELECT custom_title, sort_title FROM games WHERE id = 'g1'")
            .fetch_one(&engine.pool)
            .await
            .unwrap();
    assert_eq!(row.0, None);
    assert_eq!(row.1, "slu1654");
}

#[tokio::test]
async fn empty_custom_title_clears_override() {
    let pool = db::connect_in_memory().await.unwrap();
    let engine = Engine::with_pool(pool).await.unwrap();
    let profile = engine.ensure_default_profile().await.unwrap();

    sqlx::query(
        "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, added_at)
         VALUES ('g1', ?, 'SLU1654', 'slu1654', 'ps2', '/roms/ps2/SLU1654.iso', '2026-01-01T00:00:00Z')",
    )
    .bind(&profile.id)
    .execute(&engine.pool)
    .await
    .unwrap();

    engine.set_custom_title("g1", Some("Renamed")).await.unwrap();
    engine.set_custom_title("g1", Some("   ")).await.unwrap(); // whitespace-only => clear
    let custom: Option<String> =
        sqlx::query_scalar("SELECT custom_title FROM games WHERE id = 'g1'")
            .fetch_one(&engine.pool)
            .await
            .unwrap();
    assert_eq!(custom, None);
}

#[tokio::test]
async fn rescan_preserves_custom_title_and_its_sort_key() {
    let pool = db::connect_in_memory().await.unwrap();
    let engine = Engine::with_pool(pool).await.unwrap();
    let profile = engine.ensure_default_profile().await.unwrap();

    let mut report = ScanReport::default();

    // First index (insert path). insert_game takes a transaction-backed
    // connection, mirroring the scanner (`self.pool.begin()` -> `&mut tx`).
    let mut tx = engine.pool.begin().await.unwrap();
    let id = engine
        .insert_game(
            &mut tx,
            &profile.id,
            "SLU1654",
            "ps2",
            "/roms/ps2/SLU1654.iso",
            123,
            "2026-01-01T00:00:00Z",
            &mut report,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    engine
        .set_custom_title(&id, Some("Final Fantasy X"))
        .await
        .unwrap();

    // Re-index the same ROM (same profile_id + rom_path) -> upsert path.
    let mut tx = engine.pool.begin().await.unwrap();
    engine
        .insert_game(
            &mut tx,
            &profile.id,
            "SLU1654",
            "ps2",
            "/roms/ps2/SLU1654.iso",
            456,
            "2026-02-01T00:00:00Z",
            &mut report,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let row: (Option<String>, String, String, i64) = sqlx::query_as(
        "SELECT custom_title, sort_title, title, file_size FROM games WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&engine.pool)
    .await
    .unwrap();
    assert_eq!(row.0.as_deref(), Some("Final Fantasy X")); // override kept
    assert_eq!(row.1, "final fantasy x"); // custom sort key kept
    assert_eq!(row.2, "SLU1654"); // derived title still refreshed
    assert_eq!(row.3, 456); // other derived fields still refreshed
}
```

Note: `insert_game` and `ScanReport` are in scope inside `library.rs`; the test module already does `use crate::{db, Engine};` — add `use super::ScanReport;` (or reference it as the crate path it lives under) to the test module's `use` list if not already imported. `insert_game` is a private method on `Engine`, callable from this in-crate test module.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p dreamvault custom_title`
Expected: FAIL — `set_custom_title` does not exist (compile error), and `rescan_preserves...` would fail because the current upsert overwrites `sort_title`.

- [ ] **Step 3: Guard the upsert against clobbering a custom sort key**

In `core/dreamvault/src/library.rs`, change the `ON CONFLICT(profile_id, rom_path) DO UPDATE SET` block inside `insert_game` (currently lines 417-421) to:

```rust
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

(`title` keeps refreshing as the fallback; `sort_title` is only refreshed from the scan when there is no override.)

- [ ] **Step 4: Implement `set_custom_title`**

In `core/dreamvault/src/library.rs`, add this method next to `set_favorite` (after line 580):

```rust
    /// Set or clear a game's user display-name override. A non-empty (trimmed)
    /// name becomes `custom_title` and drives `sort_title` (so sort and search
    /// follow the new name). Passing `None` or a whitespace-only name clears the
    /// override and reverts `sort_title` to the scanned `title`'s key.
    pub async fn set_custom_title(&self, game_id: &str, title: Option<&str>) -> Result<()> {
        let trimmed = title.map(str::trim).filter(|s| !s.is_empty());
        match trimmed {
            Some(name) => {
                sqlx::query("UPDATE games SET custom_title = ?, sort_title = ? WHERE id = ?")
                    .bind(name)
                    .bind(sort_key(name))
                    .bind(game_id)
                    .execute(&self.pool)
                    .await?;
            }
            None => {
                let derived: Option<String> =
                    sqlx::query_scalar("SELECT title FROM games WHERE id = ?")
                        .bind(game_id)
                        .fetch_optional(&self.pool)
                        .await?;
                if let Some(derived) = derived {
                    sqlx::query(
                        "UPDATE games SET custom_title = NULL, sort_title = ? WHERE id = ?",
                    )
                    .bind(sort_key(&derived))
                    .bind(game_id)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        Ok(())
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p dreamvault custom_title`
Expected: PASS (all three tests).

- [ ] **Step 6: Run the full crate test suite to check for regressions**

Run: `cargo test -p dreamvault`
Expected: PASS (no existing test broken by the upsert change).

- [ ] **Step 7: Commit**

```bash
git add core/dreamvault/src/library.rs
git commit -m "feat(library): set_custom_title with rescan-safe sort key"
```

---

### Task 3: Surface custom names in Stats (most-played / recently-played)

**Files:**
- Modify: `core/dreamvault/src/stats.rs:391-413` (the two `GamePlaytime` queries)
- Test: `core/dreamvault/src/library.rs` or a test in `stats.rs` (see step 1)

**Interfaces:**
- Consumes: `games.custom_title` (Task 1).
- Produces: `GamePlaytime.title` now reflects the effective display name (custom if set, else derived). No struct or frontend change — the SQL resolves it.

- [ ] **Step 1: Write the failing test**

`stats.rs` has no test module today. Add a `library.rs`-style test to the `mod tests` block in `core/dreamvault/src/library.rs` (it can reach the stats method through `engine`):

```rust
#[tokio::test]
async fn stats_most_played_uses_custom_title() {
    let pool = db::connect_in_memory().await.unwrap();
    let engine = Engine::with_pool(pool).await.unwrap();
    let profile = engine.ensure_default_profile().await.unwrap();

    sqlx::query(
        "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, playtime_minutes, added_at)
         VALUES ('g1', ?, 'SLU1654', 'slu1654', 'ps2', '/roms/ps2/SLU1654.iso', 120, '2026-01-01T00:00:00Z')",
    )
    .bind(&profile.id)
    .execute(&engine.pool)
    .await
    .unwrap();
    engine.set_custom_title("g1", Some("Final Fantasy X")).await.unwrap();

    let stats = engine.library_stats(&profile.id).await.unwrap();
    assert_eq!(stats.most_played[0].title, "Final Fantasy X");
}
```

Confirm the stats method name: it is invoked from the Tauri layer as `library_stats` (see `desktop/src/api/commands.ts` `libraryStats` -> command `library_stats`). If the engine method has a different name, match it; grep `pub async fn` in `stats.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p dreamvault stats_most_played_uses_custom_title`
Expected: FAIL — `assert_eq!` shows `"SLU1654"` instead of `"Final Fantasy X"`.

- [ ] **Step 3: Resolve the effective title in SQL**

In `core/dreamvault/src/stats.rs`, in **both** `GamePlaytime` queries (`most_played` and `recently_played`), replace the bare `title` in the `SELECT` list with a resolved alias:

```sql
            SELECT id, COALESCE(NULLIF(custom_title, ''), title) AS title,
                   platform, cover_art, playtime_minutes, launch_count, last_played
```

Apply the identical change to both `SELECT ... FROM games` statements.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p dreamvault stats_most_played_uses_custom_title`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/dreamvault/src/stats.rs core/dreamvault/src/library.rs
git commit -m "feat(stats): resolve custom_title in playtime queries"
```

---

### Task 4: Tauri command + handler registration

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs` (add after `set_favorite`, ~line 187)
- Modify: `desktop/src-tauri/src/main.rs:104+` (the `generate_handler!` list; `set_favorite` is registered around line 123)

**Interfaces:**
- Consumes: `Engine::set_custom_title` (Task 2).
- Produces: Tauri command `set_custom_title` taking `gameId: String, title: Option<String>` (JS camelCase `gameId` maps to Rust `game_id`).

- [ ] **Step 1: Add the command**

In `desktop/src-tauri/src/commands.rs`, directly after the `set_favorite` command (line 187):

```rust
#[tauri::command]
pub async fn set_custom_title(
    state: State<'_, AppState>,
    game_id: String,
    title: Option<String>,
) -> CmdResult<()> {
    map(state
        .engine
        .set_custom_title(&game_id, title.as_deref())
        .await)
}
```

- [ ] **Step 2: Register the handler**

In `desktop/src-tauri/src/main.rs`, add `commands::set_custom_title,` to the `tauri::generate_handler![ ... ]` list, immediately after the `commands::set_favorite,` line (~123).

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p arcadia` (or the desktop crate name; if unsure run `cargo build` from repo root).
Expected: compiles with no errors.

- [ ] **Step 4: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/main.rs
git commit -m "feat(tauri): expose set_custom_title command"
```

---

### Task 5: Frontend plumbing — type, command wrapper, display helper

**Files:**
- Modify: `desktop/src/api/types.ts:51` (add field to `Game`)
- Modify: `desktop/src/api/commands.ts:82` (add `setCustomTitle` after `setFavorite`)
- Create: `desktop/src/lib/game.ts` (the `displayTitle` helper)
- Modify: `desktop/src/components/GameCard.tsx` (use `displayTitle`)

**Interfaces:**
- Consumes: the `set_custom_title` command (Task 4); `Game.custom_title` from the backend (Task 1).
- Produces:
  - `Game.custom_title: string | null` (TS type).
  - `api.setCustomTitle(gameId: string, title: string | null): Promise<void>`.
  - `displayTitle(game: { custom_title: string | null; title: string }): string` exported from `desktop/src/lib/game.ts`.

- [ ] **Step 1: Add the type field**

In `desktop/src/api/types.ts`, in the `Game` type, add after `title: string;` (line 51):

```ts
  title: string;
  custom_title: string | null;
  sort_title: string;
```

- [ ] **Step 2: Add the command wrapper**

In `desktop/src/api/commands.ts`, after the `setFavorite` entry (line 82):

```ts
  setCustomTitle: (gameId: string, title: string | null) =>
    invoke<void>("set_custom_title", { gameId, title }),
```

- [ ] **Step 3: Create the display helper**

Create `desktop/src/lib/game.ts`:

```ts
import type { Game } from "../api/types";

/** The name to show for a game: the user override if set, else the scanned title. */
export function displayTitle(game: Pick<Game, "custom_title" | "title">): string {
  return game.custom_title || game.title;
}
```

- [ ] **Step 4: Use the helper in GameCard**

In `desktop/src/components/GameCard.tsx`:
- Add the import near the other `../lib` import: `import { displayTitle } from "../lib/game";`
- Replace the three `game.title` usages (`ariaLabel`, `alt`, and the `<div className="truncate ...">{game.title}</div>`) with `displayTitle(game)`:

```tsx
      ariaLabel={displayTitle(game)}
```
```tsx
            alt={displayTitle(game)}
```
```tsx
        <div className="truncate text-sm font-semibold">{displayTitle(game)}</div>
```

- [ ] **Step 5: Typecheck/build**

Run: `cd desktop && npm run build`
Expected: `tsc --noEmit` passes and vite build succeeds.

- [ ] **Step 6: Commit**

```bash
git add desktop/src/api/types.ts desktop/src/api/commands.ts desktop/src/lib/game.ts desktop/src/components/GameCard.tsx
git commit -m "feat(ui): add custom_title type, command, and displayTitle helper"
```

---

### Task 6: Rename UI in GameDetail header

**Files:**
- Modify: `desktop/src/views/GameDetail.tsx` — imports (top), the title `<h1>` (line 282), the cover `alt`/`placeholder` (lines 215, 235), and add a rename mutation alongside the existing mutations (near `launch`/`suggest`).

**Interfaces:**
- Consumes: `api.setCustomTitle` (Task 5); `displayTitle` (Task 5); the existing `game` query (`["game", gameId]`), `qc` (`useQueryClient`), `setToast`, `Focusable`, and `useState` already imported in this file.
- Produces: an editable title in the GameDetail header with save + reset-to-original.

- [ ] **Step 1: Add imports and local edit state**

At the top of `desktop/src/views/GameDetail.tsx`, add:

```ts
import { displayTitle } from "../lib/game";
```

Inside the `GameDetail` component, near the other `useState` calls (after line 25), add:

```ts
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
```

- [ ] **Step 2: Add the rename mutation**

Near the existing `launch` mutation (after it, ~line 50), add:

```ts
  const rename = useMutation({
    mutationFn: (title: string | null) => api.setCustomTitle(gameId, title),
    onSuccess: () => {
      setEditingTitle(false);
      qc.invalidateQueries({ queryKey: ["game", gameId] });
      qc.invalidateQueries({ queryKey: ["games"] });
      setToast("Name updated");
    },
    onError: (e: unknown) => setToast(`Rename failed: ${String(e)}`),
  });
```

- [ ] **Step 3: Replace the static title with an editable header**

Replace the `<h1>` block at line 281-283:

```tsx
          <h1 className="font-display text-4xl font-black tracking-wide text-glow">
            {g.title}
          </h1>
```

with an inline editor. `g` is the loaded game object in this scope:

```tsx
          {editingTitle ? (
            <div className="flex flex-col gap-2">
              <input
                autoFocus
                value={titleDraft}
                onChange={(e) => setTitleDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") rename.mutate(titleDraft);
                  if (e.key === "Escape") setEditingTitle(false);
                }}
                aria-label="Edit display name"
                placeholder={g.title}
                className="glass w-full rounded-xl bg-surface-2 px-3 py-2 font-display text-2xl font-bold outline-none placeholder:text-ink-dim/60"
              />
              <div className="flex gap-2">
                <Focusable
                  onActivate={() => rename.mutate(titleDraft)}
                  ariaLabel="Save name"
                  className="glass rounded-xl px-3 py-1.5 text-xs font-semibold"
                >
                  {rename.isPending ? "Saving…" : "Save"}
                </Focusable>
                {g.custom_title && (
                  <Focusable
                    onActivate={() => rename.mutate(null)}
                    ariaLabel="Reset to original name"
                    className="glass rounded-xl px-3 py-1.5 text-xs font-semibold text-ink-dim"
                  >
                    Reset to original
                  </Focusable>
                )}
                <Focusable
                  onActivate={() => setEditingTitle(false)}
                  ariaLabel="Cancel rename"
                  className="glass rounded-xl px-3 py-1.5 text-xs font-semibold text-ink-dim"
                >
                  Cancel
                </Focusable>
              </div>
            </div>
          ) : (
            <div className="flex items-center gap-3">
              <h1 className="font-display text-4xl font-black tracking-wide text-glow">
                {displayTitle(g)}
              </h1>
              <Focusable
                onActivate={() => {
                  setTitleDraft(displayTitle(g));
                  setEditingTitle(true);
                }}
                ariaLabel="Edit display name"
                className="glass rounded-lg px-2 py-1 text-xs font-semibold text-ink-dim"
              >
                Edit name
              </Focusable>
            </div>
          )}
```

- [ ] **Step 4: Use the display name in cover alt/placeholder**

For consistency, update the two cover references that use `g.title`:
- Line 215: `alt={g.title}` → `alt={displayTitle(g)}`
- Line 235: `placeholder={g.title}` → `placeholder={displayTitle(g)}`

(Leave the libretro art search using the *derived* title is also acceptable, but using the display name is fine and consistent. If the box-art search behaves better with the raw title, keep line 235 as `g.title` — your call; default to `displayTitle(g)`.)

- [ ] **Step 5: Typecheck/build**

Run: `cd desktop && npm run build`
Expected: passes.

- [ ] **Step 6: Commit**

```bash
git add desktop/src/views/GameDetail.tsx
git commit -m "feat(ui): rename game display name from GameDetail header"
```

---

### Task 7: Manual verification

**Files:** none (verification only).

- [ ] **Step 1: Build and install**

Build the release binary with the custom protocol feature and atomic-install (per project convention; the binary is `dreamshell`, installed as `/usr/bin/arcadia`):

```bash
cargo build --release --features custom-protocol
cp target/release/dreamshell /usr/bin/arcadia.new && \
  SUDO_ASKPASS=~/.local/bin/sudo-askpass sudo -A mv -f /usr/bin/arcadia.new /usr/bin/arcadia
```

(Adjust the build invocation to the desktop crate if `--features custom-protocol` lives there; check `desktop/src-tauri/Cargo.toml`.)

- [ ] **Step 2: Verify behavior in the running app**

Launch Arcadia and confirm:
1. Open a game showing a coded name (e.g. `SLU1654`) → click **Edit name** → type a real title → **Save**.
2. The detail header now shows the new name; the toast "Name updated" appears.
3. Navigate back to the library grid — the card shows the new name.
4. The renamed game **sorts** under the new name (default A–Z sort) and a **search** for the new name matches it.
5. The Stats view's most-played / recently-played lists (if the game has playtime) show the new name.
6. Re-open the game, **Reset to original** → the scanned `SLU1654` name returns; sort/search revert.
7. Trigger a library rescan (if exposed in the UI) → the custom name persists.

- [ ] **Step 3: Final commit (if any cleanup needed)**

If verification surfaces no code changes, nothing to commit. Otherwise fix, re-verify, and commit with a descriptive message.

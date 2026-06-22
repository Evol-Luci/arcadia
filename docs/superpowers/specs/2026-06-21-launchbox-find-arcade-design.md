# LaunchBox + Arcade in the Cover "Find" Search — Design

**Date:** 2026-06-21
**Status:** Approved for planning
**Builds on:** the LaunchBox cover provider (commits `c2f4a7f`..`7c848b7` on `release/0.1.1`).

## Goal

Make the interactive per-game **"Find"** cover search query the LaunchBox
index (when the user has downloaded it) alongside libretro, present both
sources' matches tagged by origin, and add arcade/MAME coverage to both
sources. Auto-scan (bulk enrichment) behavior is unchanged — it stays
best-effort and is out of scope here.

## Background / Problem

Two independent gaps exist today:

1. **"Find" never consults LaunchBox.** `Engine::suggest_covers` and
   `Engine::refetch_cover` (`core/dreamvault/src/metadata.rs`) only call the
   libretro thumbnail provider. The LaunchBox index is read *only* by the bulk
   enrichment pass (`launchbox_provider()` in `enrich_metadata_online`), never
   by the interactive search. So even with the DB downloaded, Find shows zero
   LaunchBox covers.

2. **Arcade/MAME is mapped by neither source.** The libretro `system_name`
   map (`metadata.rs:169`) and the LaunchBox platform map (`launchbox.rs:29`)
   both lack an arcade entry, so arcadia's `"arcade"` platform falls through to
   `None`: arcade games are skipped on scan and return nothing on Find from
   both sources.

### Accepted limitation (scope decision)

Arcade ROMs are named by MAME/FBNeo **setname** (e.g. file `sf2.zip`, stored
title `sf2`), while libretro and LaunchBox key arcade covers by **full title**
("Street Fighter II"). Auto-scan matching for true setname-named ROMs is
therefore inherently unreliable without a setname→title database, which is
**explicitly out of scope**. The reliable arcade path is interactive Find,
where the user types the real title. (Games like "221B Baker Street
(1986)(Datasoft)" that landed in `arcade` via the ambiguous-archive fallback
already have a real title stored, so they benefit immediately.)

## Design

### 1. Platform maps — add arcade

- LaunchBox map (`launchbox.rs`, the `name => slug` match): add
  `"Arcade" => "arcade"`. Arcade entries now survive index build.
- Libretro `system_name` (`metadata.rs`): add `"arcade" => "MAME"` (libretro's
  `MAME/Named_Boxarts/` repository). Arcade Find now returns libretro hits.

### 2. LaunchBox index — add a display `name` column

Schema becomes `covers(platform, norm_name, name, file_name)`; primary key
stays `(platform, norm_name)`. `IndexRow` gains a `name` field holding the
original un-normalized title for that row (each alternate-name row stores its
own original name). `build_index_db` writes the new column.

The existing `lookup_cover(pool, platform, norm_name)` (used by auto-scan) is
**unchanged** — it still selects `file_name` by exact `norm_name`.

A new fuzzy-search function is added:

```
search_covers(pool, platform, query, limit) -> Vec<(String /*name*/, String /*file_name*/)>
```

It runs a SQL `LIKE` prefilter on `norm_name` (using the longest token of the
normalized query) to bound the candidate set, then scores candidates in Rust
with the same token-overlap scoring libretro's suggest uses, returning the top
`limit` as `(name, file_name)` pairs.

**Migration:** refresh rebuilds the whole index (refresh is the only
downloader), so the new column appears automatically after the next "Refresh
database". Anyone who downloaded during testing re-pulls once. No in-place
migration is needed.

### 3. Engine `suggest_covers` → tagged candidates

Return type changes from `Vec<String>` to `Vec<CoverCandidate>`:

```rust
pub struct CoverCandidate {
    pub source: String, // "libretro" | "launchbox"
    pub label: String,  // title to display (source tag rendered separately)
    pub token: String,  // libretro name, or LaunchBox file_name
}
```

`suggest_covers` runs the libretro suggest exactly as today (mapped into
`CoverCandidate` with `source = "libretro"`, `token = label = name`), then —
**only if** `launchbox_provider()` returns `Some` (feature enabled AND index
file exists) — runs `search_covers` and appends those rows as
`source = "launchbox"`, `label = name`, `token = file_name`. Libretro results
are listed first (primary source); LaunchBox results follow.

### 4. New apply path

New engine method and Tauri command:

```
apply_cover(game_id, source, token) -> bool
```

- `source == "libretro"` → existing `LibretroThumbnailProvider::fetch_named(game, token)`.
- `source == "launchbox"` → new `LaunchBoxProvider::fetch_file(game, file_name)`:
  download `image_url(file_name)`, cache via the existing `cache_path`, return
  a cover-only `MetadataPatch`.

Then `apply_patch`. Returns `false` when the resulting patch is empty (nothing
downloaded).

`apply_cover` **replaces** `refetch_cover`, which becomes dead code: its only
caller is the Find pick action (`GameDetail.tsx`), and there is no `None`-query
caller anywhere. Remove the now-unused `refetch_cover` end to end — the engine
method (`metadata.rs`), the Tauri command and its `generate_handler!`
registration (`commands.rs` / `main.rs`), and the `refetchCover` binding
(`commands.ts`).

### 5. Frontend

- `desktop/src/api/types.ts`: add
  `CoverCandidate { source: "libretro" | "launchbox"; label: string; token: string }`.
- `desktop/src/api/commands.ts`: `suggestCovers` now returns `CoverCandidate[]`;
  add `applyCover(gameId, source, token) => invoke<boolean>("apply_cover", …)`.
- `desktop/src/views/GameDetail.tsx`: the suggestions list renders each
  candidate's `label` plus a small source tag ("libretro" / "LaunchBox");
  activating a row calls `applyCover(c.source, c.token)` instead of the old
  `refetchCover(name)`. `suggestions` state becomes `CoverCandidate[]`.
- Remove the `refetchCover` binding from `commands.ts` (replaced by
  `applyCover`).

### 6. Error handling

All best-effort. If the LaunchBox index is absent or the feature is disabled,
`launchbox_provider()` is `None` and Find behaves exactly as today (libretro
only) — no error surfaced. A failed LaunchBox download during `apply_cover`
yields an empty patch → `false` → the existing "Couldn't download that cover —
try another." toast.

## Testing

- `launchbox.rs`:
  - `search_covers` fuzzy match: an arcade-style title query returns the right
    `(name, file_name)`.
  - `name` column round-trips through `build_index_db` / a select.
  - platform map: `"Arcade" => "arcade"`.
- `metadata.rs`:
  - libretro `system_name`: `"arcade" => "MAME"`.
  - `suggest_covers` merges and tags both sources (libretro first), gated on
    provider presence.
  - `apply_cover` routes a `launchbox` token to a cover-only patch (and a
    `libretro` token to the libretro path).
- Frontend: type-check/build (`cd desktop && npm run build`).

## Out of Scope

- MAME setname→full-title resolution for auto-scan.
- Thumbnail/image previews in the Find list (text + source tag only).
- Any change to the bulk enrichment pass or `lookup_cover` auto path.

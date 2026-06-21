# LaunchBox Games Database cover provider — design

Date: 2026-06-21
Status: Approved (design); pending implementation plan

## Goal

Add the LaunchBox Games Database (https://gamesdb.launchbox-app.com/) as an
additional source of box-art cover images, slotting in as a **fallback after the
existing libretro thumbnail source**. Cover images only — text metadata
(synopsis/genre/developer/publisher/date) stays the ScreenScraper provider's job.

## Why this is legitimate (research summary)

LaunchBox has **no public API**. The sanctioned programmatic path is the daily
metadata dump at `https://gamesdb.launchbox-app.com/Metadata.zip`. LaunchBox
founder Jason Carr explicitly endorses programmatic image access via the dump
("metadata for all of the images" with filenames that "can be easily used to
construct a URL") and notes other third-party apps already use the database with
approval. Scraping the website HTML is **not** the endorsed route and is avoided.

Source: https://forums.launchbox-app.com/topic/54163-is-there-a-public-way-to-get-images-from-the-launchbox-games-database/

## Key constraints

- The dump is the only granularity: one large `Metadata.xml` (hundreds of MB
  zipped, >1 GB unzipped). There is no per-platform or per-game download.
- Image URLs are **not derivable** — each `<GameImage>` carries a stored
  `FileName` that must come from the dump.
- Therefore the dump must be fetched to build any local name→image mapping.

## User decisions (from brainstorming)

1. **Role:** fallback after libretro (libretro stays primary).
2. **Mechanism:** the sanctioned `Metadata.zip` dump (not website scraping).
3. **Trigger:** **opt-in** — the dump only downloads when the user explicitly
   enables/refreshes LaunchBox covers; never automatically in the background.

## Architecture

A new `LaunchBoxProvider` implementing the existing `MetadataProvider` trait in
`core/dreamvault/src/metadata.rs`, cover-only like `LibretroThumbnailProvider`.
It runs as a **third enrichment pass after the libretro pass**, gated on an
opt-in config flag.

### 1. Config (opt-in)

New block in `AppConfig` (`core/dreamvault/src/config.rs`), default disabled,
following the `screenscraper` / `retroachievements` `#[serde(default)]` pattern:

```rust
pub struct LaunchBoxConfig {
    pub enabled: bool,            // default false
    pub last_refresh: Option<i64> // unix seconds of last successful index build
}
impl LaunchBoxConfig { pub fn is_enabled(&self) -> bool { self.enabled } }
```

- Disabled → the provider is a hard no-op. Zero network, ever.
- Enabled → the index is built on first use or when stale / user-refreshed.

### 2. The sidecar index

A dedicated SQLite file at `artwork_dir/launchbox/index.sqlite`, isolated from
the main app DB so a rebuild is a clean file-swap (never churns the main DB):

```
covers(platform TEXT, norm_name TEXT, file_name TEXT, PRIMARY KEY(platform, norm_name))
```

Built by stream-parsing `Metadata.xml` (`quick-xml`) and walking:
- `<Game>` → `DatabaseID`, `Name`, `Platform`
- `<GameAlternateName>` → extra names mapped to the same `DatabaseID` (so
  renamed / region titles still match)
- `<GameImage>` filtered to `Type="Box - Front"`; region preference
  World / North America → else first available.

Only platforms Arcadia supports are indexed (keeps the index small). Index build
is **atomic**: build into a temp file, swap on success — a failed or partial
refresh never corrupts a working prior index. The raw zip/XML is deleted after
indexing, so the persistent footprint is the compact index only.

### 3. The provider

`LaunchBoxProvider` implements `MetadataProvider::fetch(game)`:

1. Map the Arcadia platform slug → LaunchBox platform name via a static map
   (mirrors `ScreenScraperProvider::system_id`). Unmapped platform → empty patch.
2. If the per-ROM image is already cached on disk → return it (offline hit, same
   as libretro).
3. Normalize the game title; look up `(platform, norm_name)` in the sidecar.
4. On hit: build `https://images.launchbox-app.com/<file_name>`, download, cache
   to `artwork_dir/launchbox/<platform>/<safe>.<ext>`, return a `cover_art` patch.
5. Miss / any failure → empty patch (non-fatal).

### 4. Orchestration

In `Engine::enrich_metadata_online` (`metadata.rs`), after the existing libretro
pass and before/around the text pass, add a third pass:

- If `launchbox.is_enabled()`: ensure the index is fresh (build with progress
  events if missing/stale), then select games **still** `cover_art IS NULL` and
  run `LaunchBoxProvider` with capped polite concurrency and serialized SQLite
  writes — identical shape to the libretro `JoinSet` loop.
- If disabled: skip entirely.

### 5. UI + Tauri commands

`desktop/src/views/Settings.tsx` gains a "LaunchBox Games Database" section:
- An **enable** toggle.
- A **Refresh database** button that shows the last-refresh time and an explicit
  "several hundred MB" download warning.

New Tauri commands (in `desktop/src-tauri/src/commands.rs`, surfaced in
`desktop/src/api/commands.ts`), mirroring the screenscraper command pattern:
- `launchbox_config` → returns `LaunchBoxConfig`
- `set_launchbox_enabled(enabled)` → persists the toggle
- `refresh_launchbox_index()` → triggers download + index build, returns the
  indexed game count

### 6. Error handling

All best-effort and non-fatal, consistent with the existing providers:
- Download / unzip / XML-parse failures log at debug and skip, leaving covers
  blank — never crash enrichment.
- Streaming parse avoids holding the whole XML in memory.
- Atomic index swap guards against partial writes.
- HTTP client carries the existing `Arcadia/0.1 (+url)` user agent; caching keeps
  request volume low and polite.

### 7. Testing

Unit tests mirror the existing parsing-focused tests in `metadata.rs` (no live
HTTP):
- XML fixture → expected index rows (Game + GameImage + GameAlternateName).
- Platform-slug → LaunchBox-name mapping.
- `Box - Front` selection + region preference priority.
- Image URL construction from `FileName`.
- Title normalization and alternate-name matching.

## Scope (YAGNI)

In scope for v1: the automatic fallback enrichment pass + the opt-in toggle and
refresh command/UI.

Out of scope for v1 (natural follow-up): wiring LaunchBox into the **manual**
per-game "suggest / refetch cover" flow on Game Detail. The automatic pass and
manual flow are independent; manual stays libretro-only for now.

import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/api/dialog";
import { api, artworkUrl } from "../api/commands";
import { useStore } from "../store/useStore";
import { Focusable } from "../components/Focusable";
import { noteGameLaunched } from "../nav/spatialNav";
import { platformName, platformShort, formatPlaytime, formatRelative } from "../lib/platforms";

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
}

export function GameDetail({ gameId }: { gameId: string }) {
  const qc = useQueryClient();
  const setView = useStore((s) => s.setView);
  const setToast = useStore((s) => s.setToast);
  const profileId = useStore((s) => s.profileId);
  const coverVersion = useStore((s) => s.coverVersion);
  const bumpCoverVersion = useStore((s) => s.bumpCoverVersion);
  const [artQuery, setArtQuery] = useState("");
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [showLaunchSettings, setShowLaunchSettings] = useState(false);

  const refreshCover = () => {
    bumpCoverVersion();
    qc.invalidateQueries({ queryKey: ["game", gameId] });
    qc.invalidateQueries({ queryKey: ["games"] });
  };

  const game = useQuery({
    queryKey: ["game", gameId],
    queryFn: () => api.getGame(gameId),
  });

  const launch = useMutation({
    mutationFn: () => api.launchGame(gameId),
    onSuccess: (r) => {
      noteGameLaunched(r.session_id);
      setToast(`Launched via ${r.emulator_name}`);
      // launch_count + last_played are bumped synchronously; playtime arrives
      // later via the session-ended event handled in App.
      qc.invalidateQueries({ queryKey: ["game", gameId] });
      qc.invalidateQueries({ queryKey: ["games"] });
    },
    onError: (e: unknown) => setToast(`Launch failed: ${String(e)}`),
  });

  // Disc list for multi-disc games. Empty for single-disc titles, so the picker
  // below renders only when there's more than one disc.
  const discs = useQuery({
    queryKey: ["discs", gameId],
    queryFn: () => api.listDiscs(gameId),
  });
  const launchDisc = useMutation({
    mutationFn: (discNumber: number) => api.launchGameDisc(gameId, discNumber),
    onSuccess: (r) => {
      noteGameLaunched(r.session_id);
      setToast(`Launched via ${r.emulator_name}`);
      qc.invalidateQueries({ queryKey: ["game", gameId] });
      qc.invalidateQueries({ queryKey: ["games"] });
    },
    onError: (e: unknown) => setToast(`Launch failed: ${String(e)}`),
  });

  const favorite = useMutation({
    mutationFn: (fav: boolean) => api.setFavorite(gameId, fav),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["game", gameId] });
      qc.invalidateQueries({ queryKey: ["games"] });
    },
  });


  const collections = useQuery({
    queryKey: ["collection-summaries", profileId],
    queryFn: () => api.collectionSummaries(profileId!),
    enabled: profileId !== null,
  });
  const memberOf = useQuery({
    queryKey: ["game-collections", gameId],
    queryFn: () => api.gameCollections(gameId),
  });

  const invalidateMembership = (collectionId: string) => {
    qc.invalidateQueries({ queryKey: ["game-collections", gameId] });
    qc.invalidateQueries({ queryKey: ["collection-summaries", profileId] });
    qc.invalidateQueries({ queryKey: ["collection-games", collectionId] });
  };

  const toggleMembership = useMutation({
    mutationFn: ({ collectionId, member }: { collectionId: string; member: boolean }) =>
      member
        ? api.removeGameFromCollection(collectionId, gameId)
        : api.addGameToCollection(collectionId, gameId),
    onSuccess: (_r, { collectionId }) => invalidateMembership(collectionId),
  });

  const removeGame = useMutation({
    mutationFn: () => api.removeGame(gameId),
    onSuccess: () => {
      setToast("Removed from library.");
      qc.invalidateQueries({ queryKey: ["games"] });
      setView("library");
    },
    onError: (e: unknown) => setToast(`Couldn't remove: ${String(e)}`),
  });

  const saveBackups = useQuery({
    queryKey: ["save-backups", gameId],
    queryFn: () => api.listSaveBackups(gameId),
  });
  const refreshBackups = () =>
    qc.invalidateQueries({ queryKey: ["save-backups", gameId] });

  const backupSaves = useMutation({
    mutationFn: () => api.backupGameSaves(gameId, null),
    onSuccess: (b) => {
      setToast(
        b
          ? `Backed up ${b.file_count} save file${b.file_count === 1 ? "" : "s"}.`
          : "No save files found yet for this game.",
      );
      refreshBackups();
    },
    onError: (e: unknown) => setToast(`Backup failed: ${String(e)}`),
  });

  const restoreSaves = useMutation({
    mutationFn: (backupId: string) => api.restoreSaveBackup(backupId),
    onSuccess: (snapshot) => {
      setToast(
        snapshot
          ? "Restored. Your previous saves were snapshotted first."
          : "Restored.",
      );
      refreshBackups();
    },
    onError: (e: unknown) => setToast(`Restore failed: ${String(e)}`),
  });

  const deleteBackup = useMutation({
    mutationFn: (backupId: string) => api.deleteSaveBackup(backupId),
    onSuccess: () => refreshBackups(),
    onError: (e: unknown) => setToast(`Couldn't delete backup: ${String(e)}`),
  });

  const pickCover = useMutation({
    mutationFn: async () => {
      const file = await open({
        multiple: false,
        directory: false,
        title: "Choose box art image",
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }],
      });
      if (typeof file === "string") return api.setGameCover(gameId, file);
      return null;
    },
    onSuccess: (r) => {
      if (r) {
        setToast("Box art updated.");
        refreshCover();
      }
    },
    onError: (e: unknown) => setToast(`Couldn't set box art: ${String(e)}`),
  });

  const suggest = useMutation({
    mutationFn: (query: string) => api.suggestCovers(gameId, query.trim() || null),
    onSuccess: (list) => {
      setSuggestions(list);
      if (list.length === 0)
        setToast("No close matches on libretro — try different words or pick a local file.");
    },
    onError: (e: unknown) => setToast(`Search failed: ${String(e)}`),
  });

  const applyCover = useMutation({
    mutationFn: (name: string) => api.refetchCover(gameId, name),
    onSuccess: (found) => {
      setSuggestions([]);
      if (found) {
        setToast("Box art updated.");
        refreshCover();
      } else {
        setToast("Couldn't download that cover — try another.");
      }
    },
    onError: (e: unknown) => setToast(`Couldn't set cover: ${String(e)}`),
  });

  const g = game.data;
  if (!g) return <p className="text-ink-dim">Loading…</p>;

  const cover = artworkUrl(g.cover_art, coverVersion);

  return (
    <div className="animate-fade-up">
      <Focusable
        onActivate={() => setView("library")}
        ariaLabel="Back"
        className="mb-4 inline-block rounded-lg px-3 py-1.5 text-sm text-ink-dim"
      >
        ‹ Back to Library
      </Focusable>

      <div className="flex gap-8">
        <div className="w-[260px] shrink-0">
          <div className="glass overflow-hidden rounded-3xl">
            <div className="aspect-[3/4] bg-surface-2">
              {cover ? (
                <img src={cover} alt={g.title} className="h-full w-full object-contain" />
              ) : (
                <div className="flex h-full w-full items-center justify-center font-display text-6xl font-black text-primary/30">
                  {platformShort(g.platform)}
                </div>
              )}
            </div>
          </div>

          {/* Box-art override: search libretro's catalog and pick a match, or
              set a local image. ROM filenames often omit subtitles / use
              different naming, so a fuzzy search beats exact matching. */}
          <div className="mt-3 flex flex-col gap-2">
            <div className="flex gap-2">
              <input
                value={artQuery}
                onChange={(e) => setArtQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") suggest.mutate(artQuery);
                }}
                placeholder={g.title}
                aria-label="Search libretro box art"
                className="glass min-w-0 flex-1 rounded-xl bg-surface-2 px-3 py-2 text-xs outline-none placeholder:text-ink-dim/60"
              />
              <Focusable
                onActivate={() => suggest.mutate(artQuery)}
                ariaLabel="Find box art matches"
                className="glass shrink-0 rounded-xl px-3 py-2 text-xs font-semibold"
              >
                {suggest.isPending ? "…" : "Find"}
              </Focusable>
            </div>

            {suggestions.length > 0 && (
              <ul className="flex max-h-72 flex-col gap-1 overflow-y-auto">
                {suggestions.map((name) => (
                  <Focusable
                    key={name}
                    onActivate={() => applyCover.mutate(name)}
                    ariaLabel={`Use ${name}`}
                    className="glass rounded-lg px-2.5 py-1.5 text-left text-[11px] leading-snug"
                  >
                    {name}
                  </Focusable>
                ))}
              </ul>
            )}

            <Focusable
              onActivate={() => pickCover.mutate()}
              ariaLabel="Set box art from file"
              className="glass rounded-xl px-3 py-2 text-center text-xs font-semibold"
            >
              {pickCover.isPending ? "Setting…" : "Set local image…"}
            </Focusable>
            <p className="text-[11px] leading-snug text-ink-dim/80">
              Missing or wrong art? Search libretro and pick the right cover, or
              set a local image.
            </p>
          </div>
        </div>

        <div className="flex-1">
          <div className="mb-1 text-xs uppercase tracking-widest text-primary">
            {platformName(g.platform)}
          </div>
          <h1 className="font-display text-4xl font-black tracking-wide text-glow">
            {g.title}
          </h1>

          <div className="mt-3 flex gap-6 text-sm text-ink-dim">
            <span>{formatPlaytime(g.playtime_minutes)}</span>
            <span>{g.launch_count} launches</span>
            <span>Last: {formatRelative(g.last_played)}</span>
          </div>

          <div className="mt-6 flex gap-3">
            <Focusable
              onActivate={() => launch.mutate()}
              ariaLabel="Play"
              className="rounded-xl bg-primary px-7 py-3 font-display text-lg font-bold text-black shadow-glow"
            >
              {launch.isPending ? "Launching…" : "▶ Play"}
            </Focusable>
            <Focusable
              onActivate={() => favorite.mutate(!g.favorite)}
              ariaLabel="Toggle favorite"
              className={`glass rounded-xl px-5 py-3 text-lg ${
                g.favorite ? "text-secondary" : "text-ink-dim"
              }`}
            >
              {g.favorite ? "★ Favorited" : "☆ Favorite"}
            </Focusable>
          </div>

          {discs.data && discs.data.length > 1 && (
            <div className="mt-6 max-w-xl">
              <h2 className="mb-2 font-display text-sm font-bold uppercase tracking-wider text-ink-dim">
                {discs.data.length} discs · ▶ Play boots the full set — swap discs from the emulator's disc menu
              </h2>
              <div className="flex flex-wrap gap-2">
                {discs.data.map((d) => (
                  <Focusable
                    key={d.disc_number}
                    onActivate={() => launchDisc.mutate(d.disc_number)}
                    ariaLabel={`Play disc ${d.disc_number}`}
                    className="glass rounded-full px-3.5 py-1.5 text-xs font-semibold text-ink-dim"
                  >
                    {launchDisc.isPending && launchDisc.variables === d.disc_number
                      ? "Launching…"
                      : `▶ ${d.label ?? `Disc ${d.disc_number}`}`}
                  </Focusable>
                ))}
              </div>
            </div>
          )}

          {g.description && (
            <p className="mt-6 max-w-xl text-sm leading-relaxed text-ink-dim">
              {g.description}
            </p>
          )}

          <div className="mt-8 max-w-xl">
            <Focusable
              onActivate={() => setShowLaunchSettings(true)}
              ariaLabel="Open launch settings"
              className="glass inline-flex items-center gap-2 rounded-xl px-4 py-2 text-xs font-semibold text-ink-dim"
            >
              ⚙ Launch Settings
            </Focusable>
            <p className="mt-1.5 text-[11px] leading-snug text-ink-dim/70">
              Choose the emulator and pass extra flags or environment variables
              for games that need special settings to run.
            </p>
          </div>

          {(collections.data ?? []).length > 0 && (
            <div className="mt-8 max-w-xl">
              <h2 className="mb-2 font-display text-sm font-bold uppercase tracking-wider text-ink-dim">
                Collections
              </h2>
              <div className="flex flex-wrap gap-2">
                {(collections.data ?? []).map((c) => {
                  const member = (memberOf.data ?? []).includes(c.id);
                  return (
                    <Focusable
                      key={c.id}
                      onActivate={() =>
                        toggleMembership.mutate({ collectionId: c.id, member })
                      }
                      ariaLabel={`${member ? "Remove from" : "Add to"} ${c.name}`}
                      className={`rounded-full px-3.5 py-1.5 text-xs font-semibold ${
                        member ? "bg-primary text-black" : "glass text-ink-dim"
                      }`}
                    >
                      {member ? "✓ " : "+ "}
                      {c.name}
                    </Focusable>
                  );
                })}
              </div>
            </div>
          )}

          <div className="mt-8 max-w-xl">
            <div className="mb-2 flex items-center justify-between">
              <h2 className="font-display text-sm font-bold uppercase tracking-wider text-ink-dim">
                Save Data
              </h2>
              <Focusable
                onActivate={() => backupSaves.mutate()}
                ariaLabel="Back up saves now"
                className="glass rounded-lg px-3 py-1.5 text-xs font-semibold"
              >
                {backupSaves.isPending ? "Backing up…" : "Back up now"}
              </Focusable>
            </div>
            {(saveBackups.data ?? []).length === 0 ? (
              <p className="text-[11px] leading-snug text-ink-dim/80">
                No backups yet. "Back up now" snapshots this game's battery saves
                and savestates. Restoring a backup always snapshots your current
                saves first, so nothing is ever overwritten without a copy.
              </p>
            ) : (
              <ul className="flex flex-col gap-1.5">
                {saveBackups.data!.map((b) => (
                  <li
                    key={b.id}
                    className="glass flex items-center gap-3 rounded-xl p-2.5 text-xs"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="font-semibold">
                        {b.kind === "pre-restore" ? "Auto snapshot" : "Backup"}
                        <span className="ml-2 font-normal text-ink-dim">
                          {b.file_count} file{b.file_count === 1 ? "" : "s"} ·{" "}
                          {formatBytes(b.byte_size)}
                        </span>
                      </div>
                      <div className="text-[11px] text-ink-dim">
                        {formatRelative(b.created_at)}
                      </div>
                    </div>
                    <Focusable
                      onActivate={() => restoreSaves.mutate(b.id)}
                      ariaLabel="Restore this backup"
                      className="rounded-lg px-2.5 py-1 font-semibold text-primary"
                    >
                      Restore
                    </Focusable>
                    <Focusable
                      onActivate={() => deleteBackup.mutate(b.id)}
                      ariaLabel="Delete this backup"
                      className="rounded-lg px-2.5 py-1 text-ink-dim"
                    >
                      Delete
                    </Focusable>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <GameSaveStates gameId={gameId} />

          <GameHotkeys gameId={gameId} />

          <GameAchievements gameId={gameId} platform={g.platform} title={g.title} />

          <GameScreenshots gameId={gameId} />

          <div className="mt-8 font-mono text-[11px] text-ink-dim/70">
            {g.rom_path}
          </div>

          <div className="mt-6">
            <Focusable
              onActivate={() => removeGame.mutate()}
              ariaLabel="Remove from library"
              className="glass rounded-xl px-4 py-2 text-xs font-semibold text-ink-dim"
            >
              {removeGame.isPending ? "Removing…" : "Remove from library"}
            </Focusable>
            <p className="mt-1.5 text-[11px] leading-snug text-ink-dim/70">
              Removes this game from Arcadia's index only. The ROM file on disk
              is left untouched.
            </p>
          </div>
        </div>
      </div>

      {showLaunchSettings && (
        <LaunchSettingsDrawer
          gameId={gameId}
          platform={g.platform}
          emulatorId={g.emulator_id}
          onClose={() => setShowLaunchSettings(false)}
        />
      )}
    </div>
  );
}

/// Slide-over drawer holding all per-game launch configuration: the emulator
/// override ("Launch with") plus extra CLI args, environment variables, and an
/// optional RetroArch core. These layer on top of the resolved emulator at
/// launch — the override picks *which* emulator runs; the rest tune *how*. Empty
/// args/env/core are dropped server-side, so clearing everything reverts to
/// engine defaults. Lives in a drawer to keep the detail page uncluttered.
function LaunchSettingsDrawer({
  gameId,
  platform,
  emulatorId,
  onClose,
}: {
  gameId: string;
  platform: string;
  emulatorId: string | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);

  const emulators = useQuery({
    queryKey: ["emulators"],
    queryFn: () => api.listEmulators(),
  });
  const setEmulator = useMutation({
    mutationFn: (id: string | null) => api.setGameEmulator(gameId, id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["game", gameId] });
      qc.invalidateQueries({ queryKey: ["games"] });
    },
  });

  // Close on Escape, matching common drawer/dialog affordances.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const settings = useQuery({
    queryKey: ["game-settings", gameId],
    queryFn: () => api.getGameSettings(gameId),
  });

  const [argsText, setArgsText] = useState("");
  const [envRows, setEnvRows] = useState<{ key: string; value: string }[]>([]);
  const [core, setCore] = useState("");

  // Seed the editable form from server state on first load and after each save
  // (the invalidate→refetch returns normalised data, re-seeding the form).
  useEffect(() => {
    if (!settings.data) return;
    setArgsText(settings.data.extra_args.join("\n"));
    setEnvRows(
      Object.entries(settings.data.env_vars).map(([key, value]) => ({ key, value })),
    );
    setCore(settings.data.core_override ?? "");
  }, [settings.data]);

  const save = useMutation({
    mutationFn: () => {
      const extra_args = argsText
        .split("\n")
        .map((s) => s.trim())
        .filter(Boolean);
      const env_vars: Record<string, string> = {};
      for (const { key, value } of envRows) {
        const k = key.trim();
        if (k) env_vars[k] = value;
      }
      return api.setGameSettings({
        game_id: gameId,
        extra_args,
        env_vars,
        core_override: core.trim() || null,
      });
    },
    onSuccess: () => {
      setToast("Launch settings saved.");
      qc.invalidateQueries({ queryKey: ["game-settings", gameId] });
    },
    onError: (e: unknown) => setToast(`Couldn't save: ${String(e)}`),
  });

  const addEnvRow = () => setEnvRows((rows) => [...rows, { key: "", value: "" }]);
  const removeEnvRow = (i: number) =>
    setEnvRows((rows) => rows.filter((_, idx) => idx !== i));
  const updateEnvRow = (i: number, field: "key" | "value", v: string) =>
    setEnvRows((rows) =>
      rows.map((r, idx) => (idx === i ? { ...r, [field]: v } : r)),
    );

  const compatible = emulators.data ?? [];

  return (
    <div
      className="fixed inset-0 z-50 flex justify-end bg-black/70 animate-fade-up"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-label="Launch settings"
        className="flex h-full w-full max-w-md flex-col overflow-y-auto border-l border-white/10 bg-surface-1 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h2 className="font-display text-lg font-black tracking-wide text-glow">
            Launch Settings
          </h2>
          <Focusable
            onActivate={onClose}
            ariaLabel="Close launch settings"
            className="glass rounded-lg px-3 py-1.5 text-xs font-semibold text-ink-dim"
          >
            ✕ Close
          </Focusable>
        </div>
        <p className="mb-4 text-[11px] leading-snug text-ink-dim/80">
          Choose the emulator and pass extra flags or environment variables for
          games that need special settings to render or run (e.g. GPU offload,
          vsync, a specific core).
        </p>

        {/* Emulator override — picks WHICH emulator runs. Saved instantly on
            click; the Save button below only persists args/env/core. */}
        <label className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-ink-dim/80">
          Launch with
        </label>
        <div className="mb-4 flex flex-wrap gap-2">
          <Focusable
            onActivate={() => setEmulator.mutate(null)}
            ariaLabel="Automatic emulator"
            className={`rounded-full px-3.5 py-1.5 text-xs font-semibold ${
              emulatorId === null ? "bg-primary text-black" : "glass text-ink-dim"
            }`}
          >
            Auto
          </Focusable>
          {compatible.map((e) => (
            <Focusable
              key={e.id}
              onActivate={() => setEmulator.mutate(e.id)}
              ariaLabel={e.name}
              className={`rounded-full px-3.5 py-1.5 text-xs font-semibold ${
                emulatorId === e.id ? "bg-primary text-black" : "glass text-ink-dim"
              }`}
            >
              {e.name}
            </Focusable>
          ))}
        </div>

        <label className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-ink-dim/80">
          Extra arguments
        </label>
      <textarea
        value={argsText}
        onChange={(e) => setArgsText(e.target.value)}
        placeholder={"--fullscreen\n--no-vsync"}
        rows={2}
        aria-label="Extra launch arguments, one per line"
        className="glass min-h-[2.5rem] w-full resize-y rounded-xl bg-surface-2 px-3 py-2 font-mono text-xs outline-none placeholder:text-ink-dim/50"
      />
      <p className="mb-3 mt-1 text-[11px] leading-snug text-ink-dim/70">
        One argument per line. Passed verbatim (no shell), before the ROM path.
      </p>

      <label className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-ink-dim/80">
        Environment variables
      </label>
      <div className="flex flex-col gap-1.5">
        {envRows.map((row, i) => (
          <div key={i} className="flex gap-2">
            <input
              value={row.key}
              onChange={(e) => updateEnvRow(i, "key", e.target.value)}
              placeholder="KEY"
              aria-label="Environment variable name"
              className="glass min-w-0 flex-1 rounded-lg bg-surface-2 px-3 py-2 font-mono text-xs outline-none placeholder:text-ink-dim/50"
            />
            <input
              value={row.value}
              onChange={(e) => updateEnvRow(i, "value", e.target.value)}
              placeholder="value"
              aria-label="Environment variable value"
              className="glass min-w-0 flex-1 rounded-lg bg-surface-2 px-3 py-2 font-mono text-xs outline-none placeholder:text-ink-dim/50"
            />
            <Focusable
              onActivate={() => removeEnvRow(i)}
              ariaLabel={`Remove ${row.key || "variable"}`}
              className="glass shrink-0 rounded-lg px-3 py-2 text-xs font-semibold text-ink-dim"
            >
              ✕
            </Focusable>
          </div>
        ))}
        <Focusable
          onActivate={addEnvRow}
          ariaLabel="Add environment variable"
          className="glass self-start rounded-lg px-3 py-1.5 text-xs font-semibold text-ink-dim"
        >
          + Add variable
        </Focusable>
      </div>

      <label className="mb-1 mt-3 block text-[11px] font-semibold uppercase tracking-wider text-ink-dim/80">
        RetroArch core override
      </label>
      <input
        value={core}
        onChange={(e) => setCore(e.target.value)}
        placeholder="e.g. snes9x"
        aria-label="RetroArch core override"
        className="glass w-full rounded-xl bg-surface-2 px-3 py-2 font-mono text-xs outline-none placeholder:text-ink-dim/50"
      />
      <p className="mb-3 mt-1 text-[11px] leading-snug text-ink-dim/70">
        Libretro core base name (no <span className="font-mono">_libretro.so</span>).
        Only used when this game launches via RetroArch; ignored by standalone
        emulators. Falls back to the platform default ({platform}) if the named
        core isn't installed.
      </p>

        <Focusable
          onActivate={() => save.mutate()}
          ariaLabel="Save launch settings"
          className="glass mt-2 self-start rounded-xl px-4 py-2 text-xs font-semibold"
        >
          {save.isPending ? "Saving…" : "Save launch settings"}
        </Focusable>
      </div>
    </div>
  );
}

/// "Recall State" grid: the game's savestates as slot cards, each with the
/// preview thumbnail the emulator wrote (when present), its slot label, and when
/// it was last written.
///
/// Whether a card is clickable depends on the emulator: when it exposes a
/// startup-load flag (RetroArch `--entryslot`, Mupen64Plus `--savestate`,
/// PCSX2/DuckStation `-statefile`, Dolphin `-s`) the card boots the game straight
/// into that state. When it doesn't (Snes9x has no CLI savestate-load, and no
/// standalone SNES emulator does), the cards render read-only — clicking would
/// silently cold-boot and ignore the state, so we don't pretend it works.
function GameSaveStates({ gameId }: { gameId: string }) {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);
  const states = useQuery({
    queryKey: ["save-states", gameId],
    queryFn: () => api.listSaveStates(gameId),
  });
  const support = useQuery({
    queryKey: ["launch-state-support", gameId],
    queryFn: () => api.gameSupportsLaunchState(gameId),
  });
  const recall = useMutation({
    mutationFn: (slot: number) => api.launchIntoState(gameId, slot),
    onSuccess: (r) => {
      noteGameLaunched(r.session_id);
      setToast(`Recalled into ${r.emulator_name}`);
      qc.invalidateQueries({ queryKey: ["game", gameId] });
      qc.invalidateQueries({ queryKey: ["games"] });
    },
    onError: (e: unknown) => setToast(`Recall failed: ${String(e)}`),
  });
  const list = states.data ?? [];
  if (list.length === 0) return null;
  const canRecall = support.data === true;

  // The card visual is identical whether or not it's launchable; only the
  // wrapper (interactive Focusable vs. static div) and hover affordance differ.
  const cardInner = (s: (typeof list)[number]) => {
    const thumb = artworkUrl(s.thumbnail);
    return (
      <>
        <div
          className={`glass aspect-[4/3] overflow-hidden rounded-xl bg-surface-2 ${
            canRecall ? "transition-shadow hover:ring-2 hover:ring-primary/60" : ""
          }`}
        >
          {thumb ? (
            <img
              src={thumb}
              alt={`${s.label} preview`}
              className="h-full w-full object-cover"
              draggable={false}
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center font-display text-2xl font-black text-primary/30">
              {s.slot < 0 ? "AUTO" : s.slot}
            </div>
          )}
        </div>
        <div className="mt-1.5 text-xs font-semibold">{s.label}</div>
        <div className="text-[11px] text-ink-dim">
          {formatRelative(s.modified_at)} · {formatBytes(s.byte_size)}
        </div>
      </>
    );
  };

  return (
    <div className="mt-8 max-w-xl">
      <h2 className="mb-2 font-display text-sm font-bold uppercase tracking-wider text-ink-dim">
        Recall State
      </h2>
      <div className="flex flex-wrap gap-3">
        {list.map((s) =>
          canRecall ? (
            <Focusable
              key={s.path}
              onActivate={() => recall.mutate(s.slot)}
              ariaLabel={`Recall ${s.label}`}
              className="w-40 rounded-xl text-left"
            >
              {cardInner(s)}
            </Focusable>
          ) : (
            <div key={s.path} className="w-40 rounded-xl text-left">
              {cardInner(s)}
            </div>
          ),
        )}
      </div>
      <p className="mt-2 text-[11px] leading-snug text-ink-dim/80">
        {canRecall
          ? "Savestates Arcadia found for this game. Click one to boot straight into it."
          : "Savestates Arcadia found for this game. This emulator can't boot into a state from the command line, so load one from its own menu."}
      </p>
    </div>
  );
}

function GameHotkeys({ gameId }: { gameId: string }) {
  const hotkeys = useQuery({
    queryKey: ["game-hotkeys", gameId],
    queryFn: () => api.gameHotkeys(gameId),
  });
  const list = hotkeys.data ?? [];
  if (list.length === 0) return null;

  return (
    <div className="mt-8 max-w-xl">
      <h2 className="mb-2 font-display text-sm font-bold uppercase tracking-wider text-ink-dim">
        Hotkeys
      </h2>
      <div className="glass flex flex-col gap-px overflow-hidden rounded-xl bg-surface-2">
        {list.map((h) => (
          <div
            key={`${h.device}-${h.action}`}
            className="flex items-center justify-between gap-4 px-3 py-2"
          >
            <span className="text-sm">{h.label}</span>
            <span className="flex flex-wrap items-center justify-end gap-1">
              {h.binding.split(" + ").map((part, i) => (
                <kbd
                  key={i}
                  className={`rounded-md border border-white/10 bg-surface-1 px-2 py-0.5 font-mono text-[11px] ${
                    h.is_default ? "text-ink-dim/70" : "text-ink"
                  }`}
                >
                  {part}
                </kbd>
              ))}
            </span>
          </div>
        ))}
      </div>
      <p className="mt-2 text-[11px] leading-snug text-ink-dim/80">
        Hotkeys for the emulator that launches this game. Dimmed keys are
        emulator defaults; bright keys are your own overrides.
      </p>
    </div>
  );
}

function GameScreenshots({ gameId }: { gameId: string }) {
  const shots = useQuery({
    queryKey: ["screenshots-game", gameId],
    queryFn: () => api.listScreenshots(gameId),
  });
  const list = shots.data ?? [];
  if (list.length === 0) return null;

  return (
    <div className="mt-8 max-w-xl">
      <h2 className="mb-2 font-display text-sm font-bold uppercase tracking-wider text-ink-dim">
        Screenshots
      </h2>
      <div className="flex flex-wrap gap-2">
        {list.map((s) => {
          const url = artworkUrl(s.path);
          return (
            url && (
              <img
                key={s.id}
                src={url}
                alt="screenshot"
                className="h-24 rounded-lg object-cover"
                draggable={false}
              />
            )
          );
        })}
      </div>
    </div>
  );
}

function GameAchievements({
  gameId,
  platform,
  title,
}: {
  gameId: string;
  platform: string;
  title: string;
}) {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);
  const [query, setQuery] = useState("");

  const configured = useQuery({
    queryKey: ["ra-configured"],
    queryFn: () => api.retroachievementsConfigured(),
  });
  const link = useQuery({
    queryKey: ["ra-link", gameId],
    queryFn: () => api.getRaLink(gameId),
    enabled: configured.data === true,
  });
  const achievements = useQuery({
    queryKey: ["achievements", gameId],
    queryFn: () => api.listAchievements(gameId),
    enabled: !!link.data,
  });
  const candidates = useQuery({
    queryKey: ["ra-search", platform, query],
    queryFn: () => api.searchRaGames(platform, query),
    enabled: false,
  });

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["ra-link", gameId] });
    qc.invalidateQueries({ queryKey: ["achievements", gameId] });
  };

  const doLink = useMutation({
    mutationFn: (raId: number) => api.linkRaGame(gameId, raId),
    onSuccess: (n) => {
      setToast(`Linked — ${n} achievement${n === 1 ? "" : "s"} cached.`);
      invalidate();
    },
    onError: (e: unknown) => setToast(`Link failed: ${String(e)}`),
  });
  const unlink = useMutation({
    mutationFn: () => api.unlinkRaGame(gameId),
    onSuccess: invalidate,
  });
  const refresh = useMutation({
    mutationFn: () => api.refreshAchievements(gameId),
    onSuccess: (n) => {
      setToast(`Refreshed — ${n} achievement${n === 1 ? "" : "s"}.`);
      invalidate();
    },
    onError: (e: unknown) => setToast(`Refresh failed: ${String(e)}`),
  });

  if (configured.data !== true) return null;

  const achs = achievements.data ?? [];
  const unlocked = achs.filter((a) => a.unlocked).length;

  return (
    <div className="mt-8 max-w-xl">
      <div className="mb-2 flex items-center justify-between">
        <h2 className="font-display text-sm font-bold uppercase tracking-wider text-ink-dim">
          Achievements
        </h2>
        {link.data && (
          <div className="flex gap-2">
            <Focusable
              onActivate={() => refresh.mutate()}
              ariaLabel="Refresh achievements"
              className="glass rounded-lg px-3 py-1.5 text-xs font-semibold"
            >
              {refresh.isPending ? "Refreshing…" : "Refresh"}
            </Focusable>
            <Focusable
              onActivate={() => unlink.mutate()}
              ariaLabel="Unlink RetroAchievements"
              className="rounded-lg px-3 py-1.5 text-xs text-ink-dim"
            >
              Unlink
            </Focusable>
          </div>
        )}
      </div>

      {!link.data ? (
        <div className="glass rounded-xl p-3">
          <p className="mb-2 text-xs text-ink-dim">
            Link this game to its RetroAchievements entry to track unlocks.
          </p>
          <div className="flex gap-2">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") candidates.refetch();
              }}
              placeholder={title}
              className="glass min-w-0 flex-1 rounded-lg bg-surface-2 px-3 py-2 text-xs outline-none placeholder:text-ink-dim/60"
            />
            <Focusable
              onActivate={() => candidates.refetch()}
              ariaLabel="Search RetroAchievements"
              className="glass shrink-0 rounded-lg px-3 py-2 text-xs font-semibold"
            >
              {candidates.isFetching ? "…" : "Search"}
            </Focusable>
          </div>
          {(candidates.data ?? []).length > 0 && (
            <ul className="mt-2 flex max-h-60 flex-col gap-1 overflow-y-auto">
              {candidates.data!.map((c) => (
                <Focusable
                  key={c.ra_game_id}
                  onActivate={() => doLink.mutate(c.ra_game_id)}
                  ariaLabel={`Link to ${c.title}`}
                  className="glass rounded-lg px-2.5 py-1.5 text-left text-[11px]"
                >
                  {c.title}
                </Focusable>
              ))}
            </ul>
          )}
        </div>
      ) : (
        <>
          <div className="mb-2 text-[11px] text-ink-dim">
            {unlocked}/{achs.length} unlocked
          </div>
          <div className="flex flex-wrap gap-2">
            {achs.map((a) => (
              <div
                key={a.id}
                title={`${a.title}${a.description ? ` — ${a.description}` : ""} (${a.points} pts)`}
                className={`h-12 w-12 overflow-hidden rounded-lg bg-surface-2 ${
                  a.unlocked ? "" : "opacity-40 grayscale"
                }`}
              >
                {a.badge_url && (
                  <img
                    src={a.badge_url}
                    alt={a.title}
                    className="h-full w-full object-cover"
                    draggable={false}
                  />
                )}
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

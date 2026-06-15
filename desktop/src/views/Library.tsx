import { useQuery } from "@tanstack/react-query";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { GameCard } from "../components/GameCard";
import { VirtualGrid } from "../components/VirtualGrid";
import { ConsolePicker, ConsoleRail } from "../components/ConsoleNav";
import { Focusable } from "../components/Focusable";
import { platformName } from "../lib/platforms";

export function Library({ profileId }: { profileId: string }) {
  const openGame = useStore((s) => s.openGame);
  const setView = useStore((s) => s.setView);
  const libraryActive = useStore((s) => s.libraryActive);
  const platform = useStore((s) => s.libraryPlatform);
  const search = useStore((s) => s.librarySearch);
  const sort = useStore((s) => s.librarySort);
  const openSection = useStore((s) => s.openLibrarySection);
  const closeSection = useStore((s) => s.closeLibrarySection);
  const setSearch = useStore((s) => s.setLibrarySearch);
  const setSort = useStore((s) => s.setLibrarySort);

  // Per-console counts for the picker / rail — cheap (a single GROUP BY), and
  // shared with Home via the same query key, so it's usually already cached.
  const stats = useQuery({
    queryKey: ["stats", profileId],
    queryFn: () => api.libraryStats(profileId),
  });

  // Games are only fetched once a section has been opened, so a large library is
  // never loaded just to reach the picker.
  const games = useQuery({
    queryKey: ["games", profileId, platform, search, sort],
    queryFn: () =>
      api.listGames({
        profile_id: profileId,
        platform,
        search: search || null,
        sort,
      }),
    enabled: libraryActive,
  });

  if (!libraryActive) {
    return (
      <ConsolePicker
        usage={stats.data?.platform_usage ?? []}
        total={stats.data?.total_games ?? 0}
        onSelect={openSection}
        onOpenSettings={() => setView("settings")}
        loading={stats.isLoading}
      />
    );
  }

  return (
    <div className="animate-fade-up">
      <header className="mb-5 flex items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <Focusable
            ariaLabel="Back to consoles"
            onActivate={closeSection}
            className="glass rounded-xl px-3 py-2 text-sm font-semibold text-ink-dim"
          >
            ‹ Consoles
          </Focusable>
          <h1 className="font-display text-3xl font-black tracking-wide text-glow">
            {platform ? platformName(platform) : "All Games"}
          </h1>
        </div>
        <div className="flex items-center gap-2">
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search…"
            className="glass w-56 rounded-xl px-4 py-2 text-sm outline-none placeholder:text-ink-dim focus:border-primary"
          />
          {[
            ["title", "A–Z"],
            ["recent", "Recent"],
            ["playtime", "Playtime"],
          ].map(([key, label]) => (
            <Chip
              key={key}
              active={sort === key}
              onClick={() => setSort(key)}
              label={label}
            />
          ))}
        </div>
      </header>

      <ConsoleRail
        usage={stats.data?.platform_usage ?? []}
        total={stats.data?.total_games ?? 0}
        active={platform}
        onSelect={openSection}
      />

      {games.data && games.data.length === 0 ? (
        <p className="mt-10 text-center text-sm text-ink-dim">
          No games match your filters.
        </p>
      ) : (
        <VirtualGrid
          items={games.data ?? []}
          getKey={(g) => g.id}
          cardWidth={180}
          cardHeight={300}
          renderItem={(g) => <GameCard game={g} onOpen={openGame} />}
        />
      )}
    </div>
  );
}

function Chip({
  active,
  onClick,
  label,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <Focusable
      onActivate={onClick}
      ariaLabel={label}
      className={`rounded-full px-3.5 py-1.5 text-xs font-semibold ${
        active ? "bg-primary text-black" : "glass text-ink-dim"
      }`}
    >
      {label}
    </Focusable>
  );
}

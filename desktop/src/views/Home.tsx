import { useQuery } from "@tanstack/react-query";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { GameRow } from "../components/GameRow";
import { EmptyState } from "../components/EmptyState";
import { PageHeader } from "../components/PageHeader";
import { formatPlaytime } from "../lib/platforms";

export function Home({ profileId }: { profileId: string }) {
  const openGame = useStore((s) => s.openGame);
  const setView = useStore((s) => s.setView);

  const recent = useQuery({
    queryKey: ["games", profileId, "recent"],
    queryFn: () =>
      api.listGames({ profile_id: profileId, sort: "recent", limit: 12 }),
  });
  const favorites = useQuery({
    queryKey: ["games", profileId, "favorites"],
    queryFn: () =>
      api.listGames({ profile_id: profileId, favorites_only: true, limit: 12 }),
  });
  const stats = useQuery({
    queryKey: ["stats", profileId],
    queryFn: () => api.libraryStats(profileId),
  });

  if (stats.data && stats.data.total_games === 0) {
    return (
      <EmptyState
        title="Your vault is empty"
        body="Point Arcadia at a folder of ROMs and run a scan. Arcadia indexes files you already have — it never downloads or relocates anything."
        actionLabel="Open Settings"
        onAction={() => setView("settings")}
      />
    );
  }

  const played = recent.data?.filter((g) => g.last_played) ?? [];

  const cards = [
    { label: "Games", value: stats.data?.total_games ?? 0 },
    {
      label: "Playtime",
      value: formatPlaytime(stats.data?.total_playtime_minutes ?? 0),
    },
    { label: "Platforms", value: stats.data?.platform_count ?? 0 },
    { label: "Favorites", value: stats.data?.favorite_count ?? 0 },
  ];

  return (
    <div className="animate-fade-up">
      <PageHeader title="Welcome back" subtitle="A world of playable memories." />

      <div className="mb-9 grid grid-cols-4 gap-3">
        {cards.map((c, i) => (
          <Stat key={c.label} label={c.label} value={c.value} index={i} />
        ))}
      </div>

      <GameRow
        title="Jump back in"
        games={played}
        onOpen={openGame}
        empty="Launch a game and it'll show up here."
      />
      <GameRow
        title="Favorites"
        games={favorites.data ?? []}
        onOpen={openGame}
        empty="Mark games with ★ to pin them here."
      />
    </div>
  );
}

function Stat({
  label,
  value,
  index = 0,
}: {
  label: string;
  value: string | number;
  index?: number;
}) {
  return (
    <div
      className="glass animate-fade-up relative overflow-hidden rounded-2xl p-4"
      style={{ animationDelay: `${index * 60}ms` }}
    >
      <span className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/60 to-transparent" />
      <div className="font-display text-2xl font-black text-primary text-glow">
        {value}
      </div>
      <div className="mt-0.5 text-xs uppercase tracking-wider text-ink-dim">
        {label}
      </div>
    </div>
  );
}

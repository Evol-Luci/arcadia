import { useQuery } from "@tanstack/react-query";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { GameCard } from "../components/GameCard";
import { VirtualGrid } from "../components/VirtualGrid";
import { PageHeader } from "../components/PageHeader";

export function Recent({ profileId }: { profileId: string }) {
  const openGame = useStore((s) => s.openGame);
  const games = useQuery({
    queryKey: ["games", profileId, "recent-full"],
    queryFn: () => api.listGames({ profile_id: profileId, sort: "recent" }),
  });

  const played = (games.data ?? []).filter((g) => g.last_played);

  return (
    <div className="animate-fade-up">
      <PageHeader
        title="Recent Games"
        subtitle="In the order you last played them."
        aside={
          played.length > 0 ? (
            <span className="glass rounded-full px-3.5 py-1.5 font-display text-sm font-bold text-primary">
              {played.length}
            </span>
          ) : undefined
        }
      />
      {played.length === 0 ? (
        <p className="text-sm text-ink-dim">
          Nothing played yet. Launch a game to start your history.
        </p>
      ) : (
        <VirtualGrid
          items={played}
          getKey={(g) => g.id}
          cardWidth={180}
          cardHeight={300}
          renderItem={(g) => <GameCard game={g} onOpen={openGame} />}
        />
      )}
    </div>
  );
}

import { useQuery } from "@tanstack/react-query";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { GameCard } from "../components/GameCard";
import { VirtualGrid } from "../components/VirtualGrid";
import { PageHeader } from "../components/PageHeader";

export function Favorites({ profileId }: { profileId: string }) {
  const openGame = useStore((s) => s.openGame);
  const games = useQuery({
    queryKey: ["games", profileId, "favorites-full"],
    queryFn: () =>
      api.listGames({ profile_id: profileId, favorites_only: true }),
  });

  const favorites = games.data ?? [];

  return (
    <div className="animate-fade-up">
      <PageHeader
        title="Favorites"
        subtitle="Every game you've starred, all in one place."
        aside={
          favorites.length > 0 ? (
            <span className="glass rounded-full px-3.5 py-1.5 font-display text-sm font-bold text-primary">
              {favorites.length}
            </span>
          ) : undefined
        }
      />
      {favorites.length === 0 ? (
        <p className="text-sm text-ink-dim">
          No favorites yet. Star a game to pin it here.
        </p>
      ) : (
        <VirtualGrid
          items={favorites}
          getKey={(g) => g.id}
          cardWidth={180}
          cardHeight={300}
          renderItem={(g) => <GameCard game={g} onOpen={openGame} />}
        />
      )}
    </div>
  );
}

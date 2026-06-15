import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { GameCard } from "../components/GameCard";
import { VirtualGrid } from "../components/VirtualGrid";
import { Focusable } from "../components/Focusable";
import { PageHeader } from "../components/PageHeader";

export function Collections({ profileId }: { profileId: string }) {
  const qc = useQueryClient();
  const openGame = useStore((s) => s.openGame);
  const setToast = useStore((s) => s.setToast);
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  const summaries = useQuery({
    queryKey: ["collection-summaries", profileId],
    queryFn: () => api.collectionSummaries(profileId),
  });

  const games = useQuery({
    queryKey: ["collection-games", selected],
    queryFn: () => api.collectionGames(selected!),
    enabled: selected !== null,
  });

  const create = useMutation({
    mutationFn: (n: string) => api.createCollection(profileId, n),
    onSuccess: () => {
      setName("");
      qc.invalidateQueries({ queryKey: ["collection-summaries", profileId] });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.removeCollection(id),
    onSuccess: (_r, id) => {
      if (selected === id) setSelected(null);
      setToast("Collection deleted.");
      qc.invalidateQueries({ queryKey: ["collection-summaries", profileId] });
    },
  });

  // Drill-down: a selected collection shows its games with a back link.
  if (selected !== null) {
    const current = (summaries.data ?? []).find((c) => c.id === selected);
    return (
      <div className="animate-fade-up">
        <Focusable
          onActivate={() => setSelected(null)}
          ariaLabel="Back to collections"
          className="mb-4 inline-block rounded-lg px-3 py-1.5 text-sm text-ink-dim"
        >
          ‹ All Collections
        </Focusable>

        <PageHeader
          title={current?.name ?? "Collection"}
          subtitle={
            current
              ? `${current.game_count} ${current.game_count === 1 ? "game" : "games"}`
              : undefined
          }
          aside={
            <Focusable
              onActivate={() => remove.mutate(selected)}
              ariaLabel="Delete collection"
              className="glass rounded-xl px-4 py-2 text-sm font-semibold text-ink-dim"
            >
              Delete collection
            </Focusable>
          }
        />

        {games.data && games.data.length === 0 ? (
          <p className="text-sm text-ink-dim">
            No games in this collection yet. Open a game and add it from its
            detail page.
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

  return (
    <div className="animate-fade-up">
      <PageHeader
        title="Collections"
        subtitle="Group games into Retro, Handheld, Arcade — whatever fits your shelf."
      />

      <div className="glass mb-6 flex max-w-md gap-2 rounded-2xl p-3">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && name.trim()) create.mutate(name.trim());
          }}
          placeholder="New collection name…"
          className="flex-1 bg-transparent px-2 text-sm outline-none placeholder:text-ink-dim"
        />
        <Focusable
          onActivate={() => name.trim() && create.mutate(name.trim())}
          ariaLabel="Create collection"
          className="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-black"
        >
          Create
        </Focusable>
      </div>

      {(summaries.data ?? []).length === 0 ? (
        <p className="text-sm text-ink-dim">
          No collections yet. Group games into Retro, Handheld, Arcade — whatever
          fits your shelf.
        </p>
      ) : (
        <div className="grid grid-cols-3 gap-4">
          {(summaries.data ?? []).map((c) => (
            <Focusable
              key={c.id}
              onActivate={() => setSelected(c.id)}
              ariaLabel={c.name}
              className="glass relative flex flex-col justify-between overflow-hidden rounded-2xl p-5 text-left"
            >
              <span className="pointer-events-none absolute -right-6 -top-8 h-20 w-20 rounded-full bg-primary/10 blur-2xl" />
              <span className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/50 to-transparent" />
              <div className="font-display text-lg font-bold tracking-wide">
                {c.name}
              </div>
              <div className="mt-3 inline-flex w-fit items-center gap-1.5 rounded-full bg-surface-2 px-2.5 py-1 text-[11px] font-semibold text-ink-dim ring-1 ring-primary/10">
                <span className="font-display text-primary">{c.game_count}</span>
                {c.game_count === 1 ? "game" : "games"}
              </div>
            </Focusable>
          ))}
        </div>
      )}
    </div>
  );
}

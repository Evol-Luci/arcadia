import { GameCard } from "./GameCard";
import { SectionHeader } from "./SectionHeader";
import type { Game } from "../api/types";

interface Props {
  title: string;
  games: Game[];
  onOpen: (id: string) => void;
  empty?: string;
}

export function GameRow({ title, games, onOpen, empty }: Props) {
  return (
    <section className="mb-7">
      <SectionHeader title={title} />
      {games.length === 0 ? (
        <p className="text-sm text-ink-dim">{empty ?? "Nothing here yet."}</p>
      ) : (
        <div className="flex gap-4 overflow-x-auto pb-3">
          {games.map((g) => (
            <GameCard key={g.id} game={g} onOpen={onOpen} />
          ))}
        </div>
      )}
    </section>
  );
}

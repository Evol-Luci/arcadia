import { Focusable } from "./Focusable";
import { artworkUrl } from "../api/commands";
import { useStore } from "../store/useStore";
import { platformShort, formatPlaytime } from "../lib/platforms";
import { displayTitle } from "../lib/game";
import type { Game } from "../api/types";

interface Props {
  game: Game;
  onOpen: (id: string) => void;
}

export function GameCard({ game, onOpen }: Props) {
  const coverVersion = useStore((s) => s.coverVersion);
  const cover = artworkUrl(game.cover_art, coverVersion);
  return (
    <Focusable
      ariaLabel={displayTitle(game)}
      onActivate={() => onOpen(game.id)}
      className="glass rounded-2xl overflow-hidden w-[180px] shrink-0"
    >
      <div className="relative aspect-[3/4] bg-surface-2 overflow-hidden">
        {cover ? (
          <img
            src={cover}
            alt={displayTitle(game)}
            className="h-full w-full object-contain"
            draggable={false}
            loading="lazy"
            decoding="async"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center">
            <span className="font-display text-4xl font-black text-primary/40">
              {platformShort(game.platform)}
            </span>
          </div>
        )}
        {game.favorite && (
          <div className="absolute right-2 top-2 text-secondary text-lg drop-shadow">
            ★
          </div>
        )}
        <div className="absolute left-2 top-2 rounded-md bg-black/55 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide text-primary">
          {platformShort(game.platform)}
        </div>
      </div>
      <div className="p-2.5">
        <div className="truncate text-sm font-semibold">{displayTitle(game)}</div>
        <div className="mt-0.5 text-[11px] text-ink-dim">
          {formatPlaytime(game.playtime_minutes)}
        </div>
      </div>
    </Focusable>
  );
}

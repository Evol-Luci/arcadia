import type { Game } from "../api/types";

/** The name to show for a game: the user override if set, else the scanned title. */
export function displayTitle(game: Pick<Game, "custom_title" | "title">): string {
  return game.custom_title || game.title;
}

import { useQuery } from "@tanstack/react-query";
import { api, artworkUrl } from "../api/commands";
import { useStore } from "../store/useStore";
import { Focusable } from "../components/Focusable";
import { PageHeader } from "../components/PageHeader";
import type { RaGameProgress } from "../api/types";

export function Achievements({ profileId }: { profileId: string }) {
  const setView = useStore((s) => s.setView);

  const configured = useQuery({
    queryKey: ["ra-configured"],
    queryFn: () => api.retroachievementsConfigured(),
  });

  const summary = useQuery({
    queryKey: ["ra-summary"],
    queryFn: () => api.raUserSummary(),
    enabled: configured.data === true,
  });

  const games = useQuery({
    queryKey: ["ra-linked", profileId],
    queryFn: () => api.listRaLinkedGames(profileId),
    enabled: configured.data === true,
  });

  return (
    <div className="animate-fade-up">
      <PageHeader
        title="Achievements"
        subtitle="Unlocks tracked across your library via RetroAchievements."
      />

      {configured.data === false ? (
        <div className="glass max-w-xl rounded-2xl p-6">
          <p className="mb-3 text-sm">
            Connect your RetroAchievements account to track unlocks across your
            library.
          </p>
          <p className="mb-4 text-xs leading-relaxed text-ink-dim">
            Arcadia reads your progress through the official RetroAchievements
            Web API using your own username and personal API key — it never
            awards achievements itself, and ships no keys. Add your credentials
            in Settings, then link individual games from their detail page.
          </p>
          <Focusable
            onActivate={() => setView("settings")}
            ariaLabel="Open settings"
            className="inline-block rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
          >
            Open Settings
          </Focusable>
        </div>
      ) : (
        <>
          {summary.data && (
            <div className="glass relative mb-6 flex flex-wrap items-center gap-8 overflow-hidden rounded-2xl p-5">
              <span className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/60 to-transparent" />
              <Stat label="Player" value={summary.data.username} />
              <Stat
                label="Points"
                value={summary.data.total_points.toLocaleString()}
              />
              <Stat
                label="Rank"
                value={summary.data.rank > 0 ? `#${summary.data.rank.toLocaleString()}` : "—"}
              />
            </div>
          )}

          {(games.data ?? []).length === 0 ? (
            <p className="text-sm text-ink-dim">
              No games linked yet. Open a game and link it to a RetroAchievements
              title to start tracking unlocks.
            </p>
          ) : (
            <div className="flex flex-col gap-3">
              {games.data!.map((g) => (
                <ProgressRow key={g.game_id} g={g} />
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function ProgressRow({ g }: { g: RaGameProgress }) {
  const openGame = useStore((s) => s.openGame);
  const coverVersion = useStore((s) => s.coverVersion);
  const cover = artworkUrl(g.cover_art, coverVersion);
  const pct = g.total > 0 ? Math.round((g.unlocked / g.total) * 100) : 0;
  const mastered = pct >= 100;

  return (
    <Focusable
      ariaLabel={g.title}
      onActivate={() => openGame(g.game_id)}
      className={`glass flex items-center gap-4 rounded-2xl p-3 ${
        mastered ? "ring-1 ring-primary/40" : ""
      }`}
    >
      <div className="h-16 w-12 shrink-0 overflow-hidden rounded-md bg-surface-2 ring-1 ring-primary/10">
        {cover && (
          <img src={cover} alt="" className="h-full w-full object-cover" draggable={false} />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-semibold">{g.title}</span>
          {mastered && (
            <span className="shrink-0 rounded-full bg-primary/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-primary">
              Mastered
            </span>
          )}
        </div>
        <div className="mt-1.5 h-2 overflow-hidden rounded-full bg-surface-2 ring-1 ring-primary/10">
          <div
            className={`h-full rounded-full ${
              mastered
                ? "bg-gradient-to-r from-primary to-secondary shadow-glow"
                : "bg-primary"
            }`}
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="mt-1 text-[11px] text-ink-dim">
          {g.unlocked}/{g.total} unlocked · {g.points_earned}/{g.points_total} pts
        </div>
      </div>
      <div
        className={`shrink-0 font-display text-xl font-black ${
          mastered ? "text-primary text-glow" : "text-primary"
        }`}
      >
        {pct}%
      </div>
    </Focusable>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[11px] uppercase tracking-wider text-ink-dim">{label}</div>
      <div className="font-display text-xl font-black text-glow">{value}</div>
    </div>
  );
}

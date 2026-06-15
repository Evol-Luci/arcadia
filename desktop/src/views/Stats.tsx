import { useQuery } from "@tanstack/react-query";
import { api, artworkUrl } from "../api/commands";
import { useStore } from "../store/useStore";
import { Focusable } from "../components/Focusable";
import { PageHeader } from "../components/PageHeader";
import { SectionHeader } from "../components/SectionHeader";
import { platformName, platformShort, formatPlaytime } from "../lib/platforms";

export function Stats({ profileId }: { profileId: string }) {
  const openGame = useStore((s) => s.openGame);
  const stats = useQuery({
    queryKey: ["stats", profileId],
    queryFn: () => api.libraryStats(profileId),
  });

  const s = stats.data;
  if (!s)
    return (
      <div className="animate-fade-up">
        <PageHeader title="Statistics" subtitle="Your Year in Review — always on." />
        <p className="text-sm text-ink-dim">Loading…</p>
      </div>
    );

  const maxPlatform = Math.max(1, ...s.platform_usage.map((p) => p.playtime_minutes));

  const cards = [
    { label: "Total Playtime", value: formatPlaytime(s.total_playtime_minutes) },
    { label: "Launches", value: s.total_launches },
    { label: "Games", value: s.total_games },
    { label: "Platforms", value: s.platform_count },
  ];

  return (
    <div className="animate-fade-up">
      <PageHeader title="Statistics" subtitle="Your Year in Review — always on." />

      <div className="mb-9 grid grid-cols-4 gap-3">
        {cards.map((c, i) => (
          <Big key={c.label} label={c.label} value={c.value} index={i} />
        ))}
      </div>

      <div className="grid grid-cols-2 gap-6">
        <section>
          <SectionHeader title="Most Played" />
          {s.most_played.length === 0 ? (
            <p className="text-sm text-ink-dim">No playtime recorded yet.</p>
          ) : (
            <ol className="flex flex-col gap-2">
              {s.most_played.map((g, i) => (
                <Focusable
                  key={g.id}
                  ariaLabel={g.title}
                  onActivate={() => openGame(g.id)}
                  className="glass flex items-center gap-3 rounded-xl p-2.5"
                >
                  <span
                    className={`w-6 shrink-0 text-center font-display text-lg font-black ${
                      i === 0
                        ? "text-primary text-glow"
                        : i <= 2
                          ? "text-primary/70"
                          : "text-ink-dim"
                    }`}
                  >
                    {i + 1}
                  </span>
                  <Thumb cover={g.cover_art} platform={g.platform} />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-semibold">{g.title}</div>
                    <div className="text-xs text-ink-dim">
                      {platformShort(g.platform)} · {g.launch_count} launches
                    </div>
                  </div>
                  <span className="shrink-0 text-sm font-semibold text-primary">
                    {formatPlaytime(g.playtime_minutes)}
                  </span>
                </Focusable>
              ))}
            </ol>
          )}
        </section>

        <section>
          <SectionHeader title="Platform Usage" />
          {s.platform_usage.length === 0 ? (
            <p className="text-sm text-ink-dim">No platforms yet.</p>
          ) : (
            <div className="glass flex flex-col gap-3.5 rounded-2xl p-4">
              {s.platform_usage.map((p) => (
                <div key={p.platform}>
                  <div className="mb-1.5 flex justify-between text-xs">
                    <span className="font-semibold">{platformName(p.platform)}</span>
                    <span className="text-ink-dim">
                      {p.game_count} games · {formatPlaytime(p.playtime_minutes)}
                    </span>
                  </div>
                  <div className="h-2.5 overflow-hidden rounded-full bg-surface-2 ring-1 ring-primary/10">
                    <div
                      className="h-full rounded-full bg-gradient-to-r from-primary to-secondary shadow-glow"
                      style={{
                        width: `${Math.max(4, (p.playtime_minutes / maxPlatform) * 100)}%`,
                      }}
                    />
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

function Big({
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

function Thumb({ cover, platform }: { cover: string | null; platform: string }) {
  const coverVersion = useStore((s) => s.coverVersion);
  const url = artworkUrl(cover, coverVersion);
  return (
    <div className="h-10 w-8 shrink-0 overflow-hidden rounded bg-surface-2">
      {url ? (
        <img src={url} alt="" className="h-full w-full object-cover" />
      ) : (
        <div className="flex h-full w-full items-center justify-center text-[9px] font-bold text-primary/40">
          {platformShort(platform)}
        </div>
      )}
    </div>
  );
}

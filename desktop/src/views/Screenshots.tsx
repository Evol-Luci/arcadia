import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, artworkUrl } from "../api/commands";
import { useStore } from "../store/useStore";
import { Focusable } from "../components/Focusable";
import type { Screenshot } from "../api/types";

export function Screenshots({ profileId }: { profileId: string }) {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);
  const [zoom, setZoom] = useState<Screenshot | null>(null);

  const shots = useQuery({
    queryKey: ["screenshots", profileId],
    queryFn: () => api.recentScreenshots(profileId, 200),
  });

  const scan = useMutation({
    mutationFn: () => api.scanScreenshots(profileId),
    onSuccess: (n) => {
      setToast(
        n === 0
          ? "No new screenshots found in your emulators' folders."
          : `Imported ${n} screenshot${n === 1 ? "" : "s"}.`,
      );
      qc.invalidateQueries({ queryKey: ["screenshots", profileId] });
    },
    onError: (e: unknown) => setToast(`Screenshot scan failed: ${String(e)}`),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteScreenshot(id),
    onSuccess: () => {
      setZoom(null);
      qc.invalidateQueries({ queryKey: ["screenshots", profileId] });
    },
  });

  const list = shots.data ?? [];

  return (
    <div className="animate-fade-up">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="font-display text-3xl font-black tracking-wide text-glow">
          Screenshots
        </h1>
        <Focusable
          onActivate={() => scan.mutate()}
          ariaLabel="Scan for screenshots"
          className="rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
        >
          {scan.isPending ? "Scanning…" : "Scan for screenshots"}
        </Focusable>
      </div>

      <p className="mb-5 max-w-2xl text-xs text-ink-dim">
        Arcadia indexes screenshots your emulators already saved (RetroArch and
        standalone capture folders), copying them into its own gallery cache.
        Originals are left untouched.
      </p>

      {list.length === 0 ? (
        <p className="text-sm text-ink-dim">
          No screenshots yet. Capture some in-game, then scan.
        </p>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-4">
          {list.map((s) => {
            const url = artworkUrl(s.path);
            return (
              <Focusable
                key={s.id}
                ariaLabel={`Screenshot from ${s.captured_at ?? s.created_at}`}
                onActivate={() => setZoom(s)}
                className="glass overflow-hidden rounded-2xl"
              >
                <div className="aspect-video bg-surface-2">
                  {url && (
                    <img
                      src={url}
                      alt="screenshot"
                      className="h-full w-full object-cover"
                      draggable={false}
                    />
                  )}
                </div>
                <div className="px-3 py-2 text-[11px] text-ink-dim">
                  {formatWhen(s.captured_at ?? s.created_at)}
                </div>
              </Focusable>
            );
          })}
        </div>
      )}

      {zoom && (
        <Lightbox
          shot={zoom}
          onClose={() => setZoom(null)}
          onDelete={() => remove.mutate(zoom.id)}
          deleting={remove.isPending}
        />
      )}
    </div>
  );
}

function Lightbox({
  shot,
  onClose,
  onDelete,
  deleting,
}: {
  shot: Screenshot;
  onClose: () => void;
  onDelete: () => void;
  deleting: boolean;
}) {
  const url = artworkUrl(shot.path);
  return (
    <div
      className="fixed inset-0 z-50 flex flex-col items-center justify-center bg-black/85 p-8"
      onClick={onClose}
    >
      {url && (
        <img
          src={url}
          alt="screenshot"
          className="max-h-[80vh] max-w-[90vw] rounded-xl object-contain shadow-2xl"
          onClick={(e) => e.stopPropagation()}
          draggable={false}
        />
      )}
      <div className="mt-4 flex gap-3" onClick={(e) => e.stopPropagation()}>
        <Focusable
          onActivate={onClose}
          ariaLabel="Close"
          className="glass rounded-xl px-4 py-2 text-sm font-semibold"
        >
          Close
        </Focusable>
        <Focusable
          onActivate={onDelete}
          ariaLabel="Delete screenshot"
          className="rounded-xl bg-secondary/20 px-4 py-2 text-sm font-semibold text-secondary"
        >
          {deleting ? "Deleting…" : "Delete from gallery"}
        </Focusable>
      </div>
    </div>
  );
}

function formatWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString();
}

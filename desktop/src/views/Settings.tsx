import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/api/dialog";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { Focusable } from "../components/Focusable";
import { PageHeader } from "../components/PageHeader";
import { SectionHeader } from "../components/SectionHeader";
import {
  allThemes,
  deleteCustomTheme,
  saveCustomTheme,
  type ArcadiaTheme,
} from "../theme/themes";
import type {
  Emulator,
  PlatformEmulators,
  RetroAchievementsCredentials,
  ScreenScraperCredentials,
  SyncConfig,
} from "../api/types";

export function Settings({ profileId }: { profileId: string }) {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);
  const setProfile = useStore((s) => s.setProfile);
  const themeId = useStore((s) => s.themeId);
  const setTheme = useStore((s) => s.setTheme);
  const [newProfile, setNewProfile] = useState("");

  const emulators = useQuery({
    queryKey: ["emulators"],
    queryFn: () => api.listEmulators(),
  });
  const adapters = useQuery({ queryKey: ["adapters"], queryFn: () => api.adapters() });
  const sources = useQuery({
    queryKey: ["sources", profileId],
    queryFn: () => api.listRomSources(profileId),
  });
  const profiles = useQuery({ queryKey: ["profiles"], queryFn: () => api.listProfiles() });

  const detect = useMutation({
    mutationFn: () => api.detectEmulators(),
    onSuccess: (list) => {
      setToast(`Detected ${list.length} emulator${list.length === 1 ? "" : "s"}.`);
      qc.invalidateQueries({ queryKey: ["emulators"] });
    },
  });

  const addSource = useMutation({
    mutationFn: async () => {
      const dir = await open({ directory: true, multiple: false, title: "Choose a ROM folder" });
      if (typeof dir === "string") return api.addRomSource(profileId, dir);
      return null;
    },
    onSuccess: (r) => {
      if (r) {
        setToast("ROM folder added.");
        qc.invalidateQueries({ queryKey: ["sources", profileId] });
      }
    },
    onError: (e: unknown) => setToast(`Couldn't add folder: ${String(e)}`),
  });

  const removeSource = useMutation({
    mutationFn: (id: string) => api.removeRomSource(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["sources", profileId] }),
  });

  const scan = useMutation({
    mutationFn: () => api.scanLibrary(profileId),
    onSuccess: (r) => {
      setToast(
        `Scan complete: ${r.added} added, ${r.skipped_existing} already indexed, ${r.unrecognized} skipped.`,
      );
      qc.invalidateQueries({ queryKey: ["games"] });
      qc.invalidateQueries({ queryKey: ["stats"] });
    },
  });

  const enrich = useMutation({
    mutationFn: () => api.enrichMetadata(profileId),
    onSuccess: (count) => {
      setToast(`Box art updated for ${count} game${count === 1 ? "" : "s"}.`);
      qc.invalidateQueries({ queryKey: ["games"] });
      qc.invalidateQueries({ queryKey: ["game"] });
    },
    onError: (e: unknown) => setToast(`Box art fetch failed: ${String(e)}`),
  });

  const createProfile = useMutation({
    mutationFn: (name: string) => api.createProfile(name),
    onSuccess: (p) => {
      setNewProfile("");
      setToast(`Profile "${p.name}" created.`);
      qc.invalidateQueries({ queryKey: ["profiles"] });
      setProfile(p.id);
    },
    onError: (e: unknown) => setToast(`Couldn't create profile: ${String(e)}`),
  });

  return (
    <div className="animate-fade-up max-w-3xl">
      <PageHeader
        title="Settings"
        subtitle="Emulators, libraries, accounts, and appearance."
      />

      {/* Emulators */}
      <Panel title="Emulators" subtitle="Detected on this machine. Arcadia orchestrates them; it never reimplements emulation.">
        <Focusable
          onActivate={() => detect.mutate()}
          ariaLabel="Scan for emulators"
          className="mb-3 inline-block rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
        >
          {detect.isPending ? "Scanning…" : "Scan for emulators"}
        </Focusable>

        {(emulators.data ?? []).length === 0 ? (
          <p className="text-sm text-ink-dim">
            None detected yet. {adapters.data?.length ?? 0} adapters available
            (RetroArch + standalone). Install an emulator via pacman or Flatpak,
            then scan.
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {emulators.data!.map((e) => (
              <EmulatorRow key={e.id} e={e} />
            ))}
          </ul>
        )}
      </Panel>

      {/* Default emulators per system */}
      <Panel
        title="Default Emulators"
        subtitle="Pick which emulator launches each system. Auto follows Arcadia's recommended pick; a per-game override (on a game's page) still wins over these."
      >
        <DefaultEmulatorsPanel onToast={setToast} />
      </Panel>

      {/* ROM folders */}
      <Panel title="ROM Folders" subtitle="Read in place at locations you choose. Arcadia never moves or downloads ROMs.">
        <div className="mb-3 flex gap-2">
          <Focusable
            onActivate={() => addSource.mutate()}
            ariaLabel="Add ROM folder"
            className="inline-block rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
          >
            Add folder…
          </Focusable>
          <Focusable
            onActivate={() => scan.mutate()}
            ariaLabel="Scan library"
            className="glass inline-block rounded-xl px-4 py-2 text-sm font-semibold"
          >
            {scan.isPending ? "Scanning…" : "Scan library"}
          </Focusable>
          <Focusable
            onActivate={() => enrich.mutate()}
            ariaLabel="Fetch box art"
            className="glass inline-block rounded-xl px-4 py-2 text-sm font-semibold"
          >
            {enrich.isPending ? "Fetching box art…" : "Fetch box art"}
          </Focusable>
        </div>
        <p className="mb-3 text-xs text-ink-dim">
          Box art is pulled from the open libretro thumbnail archive (no account
          needed) and cached locally. Text details (synopsis, genre, developer)
          come from ScreenScraper when you add credentials below. Large libraries
          may take a minute.
        </p>
        {(sources.data ?? []).length === 0 ? (
          <p className="text-sm text-ink-dim">No folders added.</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {sources.data!.map((src) => (
              <li
                key={src.id}
                className="glass flex items-center gap-3 rounded-xl p-3 text-sm"
              >
                <span className="flex-1 truncate font-mono text-xs">{src.path}</span>
                <Focusable
                  onActivate={() => removeSource.mutate(src.id)}
                  ariaLabel="Remove folder"
                  className="rounded-lg px-2 py-1 text-xs text-secondary"
                >
                  Remove
                </Focusable>
              </li>
            ))}
          </ul>
        )}
      </Panel>

      {/* Metadata provider credentials */}
      <Panel
        title="Metadata Provider"
        subtitle="ScreenScraper credentials are yours — Arcadia ships none. Used under your own rate quota to fetch synopsis, genre, developer, publisher, and release date. Leave blank to skip."
      >
        <ScreenScraperPanel onToast={setToast} />
      </Panel>

      {/* Appearance */}
      <Panel title="Appearance" subtitle="Theme Engine — re-skins the whole interface instantly. Author your own or import a shared theme.">
        <ThemePanel
          themeId={themeId}
          onApply={setTheme}
          onToast={setToast}
        />
      </Panel>

      {/* RetroAchievements */}
      <Panel
        title="RetroAchievements"
        subtitle="Your RA username and personal Web API key power the Achievement Hub. Arcadia ships no keys and only reads your progress. Leave blank to disable."
      >
        <RetroAchievementsPanel onToast={setToast} />
      </Panel>

      {/* Cloud Sync */}
      <Panel
        title="Cloud Sync"
        subtitle="Point Arcadia at a folder synced by Syncthing, Nextcloud, or similar. Arcadia mirrors save backups additively (never overwriting) and keeps both copies on a config conflict — it runs no sync daemon itself."
      >
        <SyncPanel onToast={setToast} />
      </Panel>

      {/* Profiles */}
      <Panel title="Workspace Profile" subtitle="Each profile scopes its own ROM folders and library. Click to switch.">
        <div className="mb-3 flex flex-wrap gap-2">
          {(profiles.data ?? []).map((p) => (
            <Focusable
              key={p.id}
              onActivate={() => {
                if (p.id !== profileId) {
                  setProfile(p.id);
                  setToast(`Switched to "${p.name}".`);
                }
              }}
              ariaLabel={`Switch to ${p.name}`}
              className={`glass rounded-full px-3.5 py-1.5 text-xs font-semibold ${
                p.id === profileId ? "text-primary ring-1 ring-primary" : "text-ink-dim"
              }`}
            >
              {p.name}
              {p.is_default ? " · default" : ""}
            </Focusable>
          ))}
        </div>
        <div className="glass flex max-w-sm gap-2 rounded-2xl p-3">
          <input
            value={newProfile}
            onChange={(e) => setNewProfile(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && newProfile.trim())
                createProfile.mutate(newProfile.trim());
            }}
            placeholder="New profile name…"
            className="flex-1 bg-transparent px-2 text-sm outline-none placeholder:text-ink-dim"
          />
          <Focusable
            onActivate={() => newProfile.trim() && createProfile.mutate(newProfile.trim())}
            ariaLabel="Create profile"
            className="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-black"
          >
            {createProfile.isPending ? "Creating…" : "Create"}
          </Focusable>
        </div>
      </Panel>
    </div>
  );
}

function EmulatorRow({ e }: { e: Emulator }) {
  const caps = [
    e.supports_saves && "saves",
    e.supports_savestates && "states",
    e.supports_achievements && "achievements",
    e.supports_screenshots && "screenshots",
  ].filter(Boolean) as string[];

  return (
    <li className="glass flex items-center gap-3 rounded-xl p-3">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-semibold">
          {e.name}
          {e.version ? <span className="ml-2 text-xs text-ink-dim">{e.version}</span> : null}
        </div>
        <div className="truncate font-mono text-[11px] text-ink-dim">
          {e.executable_path}
        </div>
        {caps.length > 0 && (
          <div className="mt-1 flex gap-1.5">
            {caps.map((c) => (
              <span
                key={c}
                className="rounded bg-primary/15 px-1.5 py-0.5 text-[10px] text-primary"
              >
                {c}
              </span>
            ))}
          </div>
        )}
      </div>
      <span
        className={`rounded-md px-2 py-1 text-[10px] font-semibold ${
          e.owner === "arcadia" ? "bg-secondary/20 text-secondary" : "bg-surface-2 text-ink-dim"
        }`}
        title={e.owner === "arcadia" ? "Arcadia manages updates" : "Managed by your package manager"}
      >
        {e.install_source}
      </span>
    </li>
  );
}

function DefaultEmulatorsPanel({ onToast }: { onToast: (msg: string) => void }) {
  const qc = useQueryClient();
  const options = useQuery({
    queryKey: ["platform-emulator-options"],
    queryFn: () => api.platformEmulatorOptions(),
  });

  const setDefault = useMutation({
    mutationFn: ({ platform, emulatorId }: { platform: string; emulatorId: string | null }) =>
      api.setDefaultEmulator(platform, emulatorId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["platform-emulator-options"] });
    },
    onError: (e: unknown) => onToast(`Couldn't set default: ${String(e)}`),
  });

  const list = options.data ?? [];
  if (list.length === 0) {
    return (
      <p className="text-sm text-ink-dim">
        No systems with an installed emulator yet. Scan for emulators above, then
        choose a default per system here.
      </p>
    );
  }

  return (
    <ul className="flex flex-col gap-3">
      {list.map((p) => (
        <PlatformEmulatorRow
          key={p.platform}
          p={p}
          onPick={(emulatorId) =>
            setDefault.mutate({ platform: p.platform, emulatorId })
          }
        />
      ))}
    </ul>
  );
}

function PlatformEmulatorRow({
  p,
  onPick,
}: {
  p: PlatformEmulators;
  onPick: (emulatorId: string | null) => void;
}) {
  // The emulator "Auto" resolves to today, so the chip can name it.
  const autoName =
    p.emulators.find((e) => e.adapter_id === p.hint_adapter_id)?.name ??
    p.emulators[0]?.name;

  return (
    <li className="glass rounded-xl p-3">
      <div className="mb-2 text-sm font-semibold">{p.name}</div>
      <div className="flex flex-wrap gap-2">
        <Focusable
          onActivate={() => onPick(null)}
          ariaLabel={`Automatic emulator for ${p.name}`}
          className={`rounded-full px-3.5 py-1.5 text-xs font-semibold ${
            p.default_emulator_id === null ? "bg-primary text-black" : "glass text-ink-dim"
          }`}
        >
          {autoName ? `Auto · ${autoName}` : "Auto"}
        </Focusable>
        {p.emulators.map((e) => (
          <Focusable
            key={e.id}
            onActivate={() => onPick(e.id)}
            ariaLabel={`Use ${e.name} for ${p.name}`}
            className={`rounded-full px-3.5 py-1.5 text-xs font-semibold ${
              p.default_emulator_id === e.id ? "bg-primary text-black" : "glass text-ink-dim"
            }`}
          >
            {e.name}
          </Focusable>
        ))}
      </div>
    </li>
  );
}

function ScreenScraperPanel({ onToast }: { onToast: (msg: string) => void }) {
  const saved = useQuery({
    queryKey: ["screenscraper-credentials"],
    queryFn: () => api.screenScraperCredentials(),
  });
  const [creds, setCreds] = useState<ScreenScraperCredentials>({
    dev_id: "",
    dev_password: "",
    user_id: "",
    user_password: "",
  });

  // Hydrate the form once the saved credentials load.
  useEffect(() => {
    if (saved.data) setCreds(saved.data);
  }, [saved.data]);

  const save = useMutation({
    mutationFn: () => api.setScreenScraperCredentials(creds),
    onSuccess: () => onToast("ScreenScraper credentials saved."),
    onError: (e: unknown) => onToast(`Couldn't save credentials: ${String(e)}`),
  });

  const field = (
    key: keyof ScreenScraperCredentials,
    label: string,
    secret: boolean,
  ) => (
    <label className="flex flex-col gap-1 text-xs">
      <span className="text-ink-dim">{label}</span>
      <input
        type={secret ? "password" : "text"}
        value={creds[key]}
        onChange={(e) => setCreds({ ...creds, [key]: e.target.value })}
        className="glass rounded-lg bg-surface-2 px-3 py-2 text-sm outline-none focus:border-primary"
        autoComplete="off"
      />
    </label>
  );

  return (
    <div className="flex max-w-md flex-col gap-3">
      <div className="grid grid-cols-2 gap-3">
        {field("dev_id", "Dev ID", false)}
        {field("dev_password", "Dev password", true)}
        {field("user_id", "Account (optional)", false)}
        {field("user_password", "Account password", true)}
      </div>
      <Focusable
        onActivate={() => save.mutate()}
        ariaLabel="Save ScreenScraper credentials"
        className="self-start rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
      >
        {save.isPending ? "Saving…" : "Save credentials"}
      </Focusable>
      <p className="text-[11px] leading-snug text-ink-dim/80">
        A developer ID/password (issued by ScreenScraper) is required; a personal
        account is optional but raises your quota. Stored locally in your config
        directory — never shared.
      </p>
    </div>
  );
}

const BLANK_THEME: ArcadiaTheme = {
  id: "",
  name: "My Theme",
  description: "Custom theme",
  className: "theme-custom",
  primary: "#00D7FF",
  secondary: "#FF4DD2",
  scanlines: false,
  crtGlow: true,
  glassOpacity: 0.6,
  custom: true,
};

function ThemePanel({
  themeId,
  onApply,
  onToast,
}: {
  themeId: string;
  onApply: (id: string) => void;
  onToast: (msg: string) => void;
}) {
  // allThemes() reads localStorage; bump to re-read after create/delete.
  const [rev, setRev] = useState(0);
  const themes = useMemo(() => allThemes(), [rev]);
  const [draft, setDraft] = useState<ArcadiaTheme | null>(null);

  const refresh = () => setRev((r) => r + 1);

  const startNew = () => setDraft({ ...BLANK_THEME });
  const startEdit = (t: ArcadiaTheme) => setDraft({ ...t });

  const saveDraft = () => {
    if (!draft) return;
    const saved = saveCustomTheme(draft);
    refresh();
    setDraft(null);
    onApply(saved.id);
    onToast(`Theme "${saved.name}" saved.`);
  };

  const remove = (t: ArcadiaTheme) => {
    deleteCustomTheme(t.id);
    refresh();
    if (themeId === t.id) onApply("dreamglass");
  };

  const exportTheme = (t: ArcadiaTheme) => {
    navigator.clipboard?.writeText(JSON.stringify(t, null, 2));
    onToast(`"${t.name}" copied as JSON — share it anywhere.`);
  };

  const importTheme = async () => {
    try {
      const text = await navigator.clipboard?.readText();
      if (!text) {
        onToast("Clipboard is empty — copy a theme's JSON first.");
        return;
      }
      const parsed = JSON.parse(text) as ArcadiaTheme;
      if (!parsed.primary || !parsed.name) {
        onToast("That doesn't look like a theme.");
        return;
      }
      const saved = saveCustomTheme({ ...parsed, id: "", custom: true });
      refresh();
      onApply(saved.id);
      onToast(`Imported "${saved.name}".`);
    } catch {
      onToast("Couldn't read a theme from the clipboard.");
    }
  };

  return (
    <div>
      <div className="mb-3 flex gap-2">
        <Focusable
          onActivate={startNew}
          ariaLabel="Create custom theme"
          className="rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
        >
          New theme
        </Focusable>
        <Focusable
          onActivate={importTheme}
          ariaLabel="Import theme from clipboard"
          className="glass rounded-xl px-4 py-2 text-sm font-semibold"
        >
          Import from clipboard
        </Focusable>
      </div>

      <div className="grid grid-cols-3 gap-3">
        {themes.map((t) => (
          <div
            key={t.id}
            className={`glass rounded-2xl p-4 ${
              themeId === t.id ? "ring-2 ring-primary" : ""
            }`}
          >
            <Focusable onActivate={() => onApply(t.id)} ariaLabel={t.name}>
              <div className="mb-2 flex gap-1.5">
                <span className="h-5 w-5 rounded-full" style={{ background: t.primary }} />
                <span className="h-5 w-5 rounded-full" style={{ background: t.secondary }} />
              </div>
              <div className="font-display font-bold">
                {t.name}
                {t.custom && (
                  <span className="ml-2 text-[10px] font-normal text-ink-dim">custom</span>
                )}
              </div>
              <div className="text-[11px] leading-snug text-ink-dim">{t.description}</div>
            </Focusable>
            <div className="mt-2 flex gap-2 text-[11px]">
              <Focusable
                onActivate={() => exportTheme(t)}
                ariaLabel={`Export ${t.name}`}
                className="text-primary"
              >
                Export
              </Focusable>
              {t.custom && (
                <>
                  <Focusable
                    onActivate={() => startEdit(t)}
                    ariaLabel={`Edit ${t.name}`}
                  >
                    Edit
                  </Focusable>
                  <Focusable
                    onActivate={() => remove(t)}
                    ariaLabel={`Delete ${t.name}`}
                    className="text-secondary"
                  >
                    Delete
                  </Focusable>
                </>
              )}
            </div>
          </div>
        ))}
      </div>

      {draft && (
        <ThemeEditor
          draft={draft}
          onChange={setDraft}
          onSave={saveDraft}
          onCancel={() => setDraft(null)}
        />
      )}
    </div>
  );
}

function ThemeEditor({
  draft,
  onChange,
  onSave,
  onCancel,
}: {
  draft: ArcadiaTheme;
  onChange: (t: ArcadiaTheme) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="glass mt-4 max-w-md rounded-2xl p-4">
      <div className="mb-3 font-display font-bold">
        {draft.id ? "Edit theme" : "New theme"}
      </div>
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-ink-dim">Name</span>
          <input
            value={draft.name}
            onChange={(e) => onChange({ ...draft, name: e.target.value })}
            className="glass rounded-lg bg-surface-2 px-3 py-2 text-sm outline-none focus:border-primary"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-ink-dim">Description</span>
          <input
            value={draft.description}
            onChange={(e) => onChange({ ...draft, description: e.target.value })}
            className="glass rounded-lg bg-surface-2 px-3 py-2 text-sm outline-none focus:border-primary"
          />
        </label>
        <div className="flex gap-4">
          <label className="flex items-center gap-2 text-xs">
            <span className="text-ink-dim">Primary</span>
            <input
              type="color"
              value={draft.primary}
              onChange={(e) => onChange({ ...draft, primary: e.target.value })}
              className="h-8 w-12 rounded bg-transparent"
            />
          </label>
          <label className="flex items-center gap-2 text-xs">
            <span className="text-ink-dim">Secondary</span>
            <input
              type="color"
              value={draft.secondary}
              onChange={(e) => onChange({ ...draft, secondary: e.target.value })}
              className="h-8 w-12 rounded bg-transparent"
            />
          </label>
        </div>
        <div className="flex flex-wrap gap-4 text-xs">
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={draft.scanlines}
              onChange={(e) => onChange({ ...draft, scanlines: e.target.checked })}
            />
            <span>Scanlines</span>
          </label>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={draft.crtGlow}
              onChange={(e) => onChange({ ...draft, crtGlow: e.target.checked })}
            />
            <span>CRT glow</span>
          </label>
          <label className="flex items-center gap-2">
            <span className="text-ink-dim">Glass</span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={draft.glassOpacity}
              onChange={(e) =>
                onChange({ ...draft, glassOpacity: Number(e.target.value) })
              }
            />
          </label>
        </div>
      </div>
      <div className="mt-4 flex gap-2">
        <Focusable
          onActivate={onSave}
          ariaLabel="Save theme"
          className="rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
        >
          Save theme
        </Focusable>
        <Focusable
          onActivate={onCancel}
          ariaLabel="Cancel"
          className="glass rounded-xl px-4 py-2 text-sm font-semibold"
        >
          Cancel
        </Focusable>
      </div>
    </div>
  );
}

function RetroAchievementsPanel({ onToast }: { onToast: (msg: string) => void }) {
  const qc = useQueryClient();
  const saved = useQuery({
    queryKey: ["ra-credentials"],
    queryFn: () => api.retroachievementsCredentials(),
  });
  const [creds, setCreds] = useState<RetroAchievementsCredentials>({
    username: "",
    api_key: "",
  });

  useEffect(() => {
    if (saved.data) setCreds(saved.data);
  }, [saved.data]);

  const save = useMutation({
    mutationFn: () => api.setRetroachievementsCredentials(creds),
    onSuccess: () => {
      onToast("RetroAchievements credentials saved.");
      qc.invalidateQueries({ queryKey: ["ra-configured"] });
      qc.invalidateQueries({ queryKey: ["ra-summary"] });
    },
    onError: (e: unknown) => onToast(`Couldn't save credentials: ${String(e)}`),
  });

  return (
    <div className="flex max-w-md flex-col gap-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-ink-dim">Username</span>
          <input
            value={creds.username}
            onChange={(e) => setCreds({ ...creds, username: e.target.value })}
            className="glass rounded-lg bg-surface-2 px-3 py-2 text-sm outline-none focus:border-primary"
            autoComplete="off"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-ink-dim">Web API key</span>
          <input
            type="password"
            value={creds.api_key}
            onChange={(e) => setCreds({ ...creds, api_key: e.target.value })}
            className="glass rounded-lg bg-surface-2 px-3 py-2 text-sm outline-none focus:border-primary"
            autoComplete="off"
          />
        </label>
      </div>
      <Focusable
        onActivate={() => save.mutate()}
        ariaLabel="Save RetroAchievements credentials"
        className="self-start rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
      >
        {save.isPending ? "Saving…" : "Save credentials"}
      </Focusable>
      <p className="text-[11px] leading-snug text-ink-dim/80">
        Find your Web API key on retroachievements.org under Settings → Keys.
        Stored locally in your config directory — never shared.
      </p>
    </div>
  );
}

function SyncPanel({ onToast }: { onToast: (msg: string) => void }) {
  const qc = useQueryClient();
  const config = useQuery({ queryKey: ["sync-config"], queryFn: () => api.syncConfig() });
  const status = useQuery({ queryKey: ["sync-status"], queryFn: () => api.syncStatus() });

  const setConfig = useMutation({
    mutationFn: (c: SyncConfig) => api.setSyncConfig(c),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["sync-config"] });
      qc.invalidateQueries({ queryKey: ["sync-status"] });
    },
  });

  const chooseFolder = useMutation({
    mutationFn: async () => {
      const dir = await open({ directory: true, multiple: false, title: "Choose sync folder" });
      if (typeof dir === "string") {
        return api.setSyncConfig({ enabled: config.data?.enabled ?? true, folder: dir });
      }
      return null;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["sync-config"] });
      qc.invalidateQueries({ queryKey: ["sync-status"] });
    },
  });

  const push = useMutation({
    mutationFn: () => api.syncPush(),
    onSuccess: (r) =>
      onToast(`Pushed ${r.files_copied} file(s); ${r.conflicts} conflict(s) kept-both.`),
    onError: (e: unknown) => onToast(`Push failed: ${String(e)}`),
  });
  const pull = useMutation({
    mutationFn: () => api.syncPull(),
    onSuccess: (r) => {
      onToast(`Pulled ${r.files_copied} file(s); ${r.conflicts} conflict(s) kept-both.`);
      qc.invalidateQueries({ queryKey: ["sync-status"] });
    },
    onError: (e: unknown) => onToast(`Pull failed: ${String(e)}`),
  });

  const cfg = config.data;
  const st = status.data;

  return (
    <div className="max-w-md">
      <div className="mb-3 flex items-center gap-3">
        <Focusable
          onActivate={() => chooseFolder.mutate()}
          ariaLabel="Choose sync folder"
          className="rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
        >
          {cfg?.folder ? "Change folder…" : "Choose folder…"}
        </Focusable>
        {cfg && (
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={cfg.enabled}
              onChange={(e) =>
                setConfig.mutate({ enabled: e.target.checked, folder: cfg.folder })
              }
            />
            <span>Enabled</span>
          </label>
        )}
      </div>

      {cfg?.folder ? (
        <div className="glass rounded-2xl p-3 text-xs">
          <div className="truncate font-mono text-ink-dim">{cfg.folder}</div>
          {st && (
            <div className="mt-2 text-ink-dim">
              {st.folder_exists ? (
                <>
                  {st.local_backup_count} local · {st.remote_backup_count} remote backups
                </>
              ) : (
                <span className="text-secondary">Folder not found.</span>
              )}
            </div>
          )}
          <div className="mt-3 flex gap-2">
            <Focusable
              onActivate={() => push.mutate()}
              ariaLabel="Push to remote"
              className="rounded-lg bg-primary/90 px-3 py-1.5 text-xs font-semibold text-black"
            >
              {push.isPending ? "Pushing…" : "Push →"}
            </Focusable>
            <Focusable
              onActivate={() => pull.mutate()}
              ariaLabel="Pull from remote"
              className="glass rounded-lg px-3 py-1.5 text-xs font-semibold"
            >
              {pull.isPending ? "Pulling…" : "← Pull"}
            </Focusable>
          </div>
        </div>
      ) : (
        <p className="text-sm text-ink-dim">
          No sync folder set. Choose one inside your synced directory.
        </p>
      )}
    </div>
  );
}

function Panel({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mb-8 border-t border-primary/10 pt-6 first:border-t-0 first:pt-0">
      <SectionHeader title={title} subtitle={subtitle} />
      {children}
    </section>
  );
}

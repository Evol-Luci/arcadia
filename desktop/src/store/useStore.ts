import { create } from "zustand";
import { applyTheme, themeById } from "../theme/themes";

export type View =
  | "home"
  | "library"
  | "recent"
  | "collections"
  | "screenshots"
  | "achievements"
  | "controller"
  | "plugins"
  | "stats"
  | "settings"
  | "game";

const THEME_KEY = "arcadia.theme";
const PROFILE_KEY = "arcadia.profile";

export interface ProgressState {
  /** "scan" | "artwork" | "metadata" */
  kind: string;
  label: string;
  done: number;
  /** 0 means indeterminate (e.g. a filesystem walk of unknown size). */
  total: number;
}

interface AppStore {
  view: View;
  profileId: string | null;
  selectedGameId: string | null;
  themeId: string;
  /** Toast-style transient message (e.g. launch feedback). */
  toast: string | null;
  /** Live progress of a background job (library scan / box-art fetch), or null
   *  when nothing is running. Driven by 'arcadia://progress' events. */
  progress: ProgressState | null;
  /** Cache-buster appended to artwork URLs. Covers are written to stable,
   *  deterministic paths (libretro cache keyed by ROM name, manual covers by
   *  game id), so correcting one overwrites the same file at the same path —
   *  the webview would otherwise serve the stale cached image. Bumping this on
   *  a cover change forces the <img> to reload. */
  coverVersion: number;

  /** Library navigation state, held here (not in the view) so it survives
   *  leaving and returning to the Library. It is in-memory only, so each app
   *  launch starts on the console picker (libraryActive = false). */
  libraryActive: boolean;
  /** Selected console slug, or null for "All". */
  libraryPlatform: string | null;
  librarySearch: string;
  librarySort: string;

  setView: (v: View) => void;
  setProfile: (id: string) => void;
  openGame: (id: string) => void;
  setTheme: (id: string) => void;
  setToast: (msg: string | null) => void;
  setProgress: (p: ProgressState | null) => void;
  bumpCoverVersion: () => void;
  /** Open a console section (or null for All) and leave the picker. */
  openLibrarySection: (platform: string | null) => void;
  /** Return to the console picker. */
  closeLibrarySection: () => void;
  setLibrarySearch: (s: string) => void;
  setLibrarySort: (s: string) => void;
}

export const useStore = create<AppStore>((set) => {
  const savedTheme = localStorage.getItem(THEME_KEY) ?? "dreamglass";
  applyTheme(themeById(savedTheme));

  return {
    view: "home",
    profileId: localStorage.getItem(PROFILE_KEY),
    selectedGameId: null,
    themeId: savedTheme,
    toast: null,
    progress: null,
    // Seed per-launch so a stale image persisted by the webview's HTTP cache is
    // bypassed on startup; in-session cover changes bump it further.
    coverVersion: Date.now(),

    libraryActive: false,
    libraryPlatform: null,
    librarySearch: "",
    librarySort: "title",

    setView: (view) => set({ view }),
    setProfile: (profileId) => {
      localStorage.setItem(PROFILE_KEY, profileId);
      set({ profileId });
    },
    openGame: (selectedGameId) => set({ selectedGameId, view: "game" }),
    setTheme: (themeId) => {
      applyTheme(themeById(themeId));
      localStorage.setItem(THEME_KEY, themeId);
      set({ themeId });
    },
    setToast: (toast) => set({ toast }),
    setProgress: (progress) => set({ progress }),
    bumpCoverVersion: () => set((s) => ({ coverVersion: s.coverVersion + 1 })),
    openLibrarySection: (libraryPlatform) =>
      set({ libraryPlatform, librarySearch: "", libraryActive: true }),
    closeLibrarySection: () => set({ libraryActive: false }),
    setLibrarySearch: (librarySearch) => set({ librarySearch }),
    setLibrarySort: (librarySort) => set({ librarySort }),
  };
});

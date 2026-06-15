import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api/commands";
import { useStore, type View } from "./store/useStore";
import { startSpatialNav, resetFocus, noteGameEnded } from "./nav/spatialNav";
import { startGamepadBridge } from "./nav/gamepadBridge";
import { Sidebar } from "./components/Sidebar";
import { Toast } from "./components/Toast";
import { Home } from "./views/Home";
import { Library } from "./views/Library";
import { Recent } from "./views/Recent";
import { Collections } from "./views/Collections";
import { Screenshots } from "./views/Screenshots";
import { Achievements } from "./views/Achievements";
import { Controller } from "./views/Controller";
import { Plugins } from "./views/Plugins";
import { Stats } from "./views/Stats";
import { Settings } from "./views/Settings";
import { GameDetail } from "./views/GameDetail";

const TAB_ORDER: View[] = [
  "home",
  "library",
  "recent",
  "collections",
  "screenshots",
  "achievements",
  "controller",
  "plugins",
  "stats",
  "settings",
];

export default function App() {
  const view = useStore((s) => s.view);
  const profileId = useStore((s) => s.profileId);
  const setProfile = useStore((s) => s.setProfile);
  const setView = useStore((s) => s.setView);
  const selectedGameId = useStore((s) => s.selectedGameId);
  const libraryActive = useStore((s) => s.libraryActive);
  const qc = useQueryClient();

  // Ensure a profile exists (the engine guarantees a default at bootstrap).
  const profile = useQuery({
    queryKey: ["default-profile"],
    queryFn: () => api.defaultProfile(),
    enabled: profileId === null,
  });
  useEffect(() => {
    if (profileId === null && profile.data) setProfile(profile.data.id);
  }, [profileId, profile.data, setProfile]);

  // Global controller / keyboard navigation.
  useEffect(() => startSpatialNav(), []);
  // Reseed focus on view change, and when the Library swaps between the console
  // picker and a loaded section (same view, different focusables).
  useEffect(() => resetFocus(), [view, libraryActive]);

  // Feed evdev gamepad state into navigator.getGamepads() (WebKitGTK's native
  // gamepad support is unreliable on Linux).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    startGamepadBridge().then((off) => {
      unlisten = off;
    });
    return () => unlisten?.();
  }, []);

  // Playtime is attributed when an emulator exits (often well after launch),
  // so the engine pushes a session-ended event. Refresh the affected queries.
  useEffect(() => {
    const unlisten = listen<{ session_id: string }>("arcadia://session-ended", (e) => {
      // The emulator has exited, so the controller is ours again — resume nav.
      if (e.payload?.session_id) noteGameEnded(e.payload.session_id);
      qc.invalidateQueries({ queryKey: ["games"] });
      qc.invalidateQueries({ queryKey: ["game"] });
      qc.invalidateQueries({ queryKey: ["stats"] });
    });
    return () => {
      unlisten.then((off) => off());
    };
  }, [qc]);

  // B button / Escape => back; bumpers => switch tabs.
  useEffect(() => {
    const onBack = () => {
      const st = useStore.getState();
      if (st.view === "game") setView("library");
      // Within a loaded console section, step back to the picker first.
      else if (st.view === "library" && st.libraryActive) st.closeLibrarySection();
      else setView("home");
    };
    const onTab = (e: Event) => {
      const delta = (e as CustomEvent<number>).detail ?? 1;
      const cur = useStore.getState().view;
      const idx = TAB_ORDER.indexOf(cur === "game" ? "library" : cur);
      const next = TAB_ORDER[(idx + delta + TAB_ORDER.length) % TAB_ORDER.length];
      setView(next);
    };
    window.addEventListener("arcadia:back", onBack);
    window.addEventListener("arcadia:tab", onTab);
    return () => {
      window.removeEventListener("arcadia:back", onBack);
      window.removeEventListener("arcadia:tab", onTab);
    };
  }, [setView]);

  if (!profileId) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="font-display text-2xl font-black text-primary text-glow">
          ARCADIA
        </div>
      </div>
    );
  }

  return (
    <div className="crt-scanlines relative flex h-full overflow-hidden">
      <Sidebar />
      <main className="flex-1 overflow-y-auto p-8">
        {view === "home" && <Home profileId={profileId} />}
        {view === "library" && <Library profileId={profileId} />}
        {view === "recent" && <Recent profileId={profileId} />}
        {view === "collections" && <Collections profileId={profileId} />}
        {view === "screenshots" && <Screenshots profileId={profileId} />}
        {view === "achievements" && <Achievements profileId={profileId} />}
        {view === "controller" && <Controller />}
        {view === "plugins" && <Plugins />}
        {view === "stats" && <Stats profileId={profileId} />}
        {view === "settings" && <Settings profileId={profileId} />}
        {view === "game" && selectedGameId && <GameDetail gameId={selectedGameId} />}
      </main>
      <Toast />
    </div>
  );
}

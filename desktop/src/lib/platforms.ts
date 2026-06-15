// Frontend-side display metadata for platforms. The engine is the source of
// truth for which platforms exist; this just maps slugs to labels and a short
// badge for the UI.

import type { PlatformUsage } from "../api/types";

interface PlatformDisplay {
  name: string;
  short: string;
  manufacturer: string;
}

const MAP: Record<string, PlatformDisplay> = {
  nes: { name: "Nintendo Entertainment System", short: "NES", manufacturer: "Nintendo" },
  snes: { name: "Super Nintendo", short: "SNES", manufacturer: "Nintendo" },
  n64: { name: "Nintendo 64", short: "N64", manufacturer: "Nintendo" },
  gamecube: { name: "GameCube", short: "GCN", manufacturer: "Nintendo" },
  wii: { name: "Wii", short: "Wii", manufacturer: "Nintendo" },
  gb: { name: "Game Boy", short: "GB", manufacturer: "Nintendo" },
  gbc: { name: "Game Boy Color", short: "GBC", manufacturer: "Nintendo" },
  gba: { name: "Game Boy Advance", short: "GBA", manufacturer: "Nintendo" },
  nds: { name: "Nintendo DS", short: "NDS", manufacturer: "Nintendo" },
  virtualboy: { name: "Virtual Boy", short: "VB", manufacturer: "Nintendo" },
  sms: { name: "Master System", short: "SMS", manufacturer: "Sega" },
  genesis: { name: "Sega Genesis", short: "GEN", manufacturer: "Sega" },
  dreamcast: { name: "Dreamcast", short: "DC", manufacturer: "Sega" },
  ps1: { name: "PlayStation", short: "PS1", manufacturer: "Sony" },
  ps2: { name: "PlayStation 2", short: "PS2", manufacturer: "Sony" },
  psp: { name: "PSP", short: "PSP", manufacturer: "Sony" },
  ps3: { name: "PlayStation 3", short: "PS3", manufacturer: "Sony" },
  vita: { name: "PlayStation Vita", short: "Vita", manufacturer: "Sony" },
};

export function platformName(slug: string): string {
  return MAP[slug]?.name ?? slug.toUpperCase();
}

export function platformShort(slug: string): string {
  return MAP[slug]?.short ?? slug.toUpperCase();
}

export function platformManufacturer(slug: string): string {
  return MAP[slug]?.manufacturer ?? "Other";
}

// Canonical (roughly chronological) ordering of consoles within a manufacturer.
const PLATFORM_ORDER = Object.keys(MAP);
function platformIndex(slug: string): number {
  const i = PLATFORM_ORDER.indexOf(slug);
  return i === -1 ? PLATFORM_ORDER.length : i;
}

const MANUFACTURER_ORDER = ["Nintendo", "Sony", "Sega"];

export interface PlatformGroup {
  manufacturer: string;
  items: PlatformUsage[];
}

/** Group per-platform usage by manufacturer for the console picker / rail.
 *  Manufacturers follow a fixed order (Nintendo, Sony, Sega, then any others
 *  alphabetically); consoles within a group follow their canonical order. */
export function groupPlatforms(usage: PlatformUsage[]): PlatformGroup[] {
  const groups = new Map<string, PlatformUsage[]>();
  for (const u of usage) {
    const m = platformManufacturer(u.platform);
    const arr = groups.get(m) ?? [];
    arr.push(u);
    groups.set(m, arr);
  }
  const extras = [...groups.keys()]
    .filter((m) => !MANUFACTURER_ORDER.includes(m))
    .sort();
  return [...MANUFACTURER_ORDER, ...extras]
    .filter((m) => groups.has(m))
    .map((manufacturer) => ({
      manufacturer,
      items: groups
        .get(manufacturer)!
        .sort((a, b) => platformIndex(a.platform) - platformIndex(b.platform)),
    }));
}

export function formatPlaytime(minutes: number): string {
  if (minutes <= 0) return "Never played";
  if (minutes < 60) return `${minutes}m`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

export function formatRelative(iso: string | null): string {
  if (!iso) return "Never";
  const then = new Date(iso).getTime();
  const diff = Date.now() - then;
  const day = 86_400_000;
  if (diff < day) return "Today";
  if (diff < 2 * day) return "Yesterday";
  if (diff < 7 * day) return `${Math.floor(diff / day)} days ago`;
  return new Date(iso).toLocaleDateString();
}

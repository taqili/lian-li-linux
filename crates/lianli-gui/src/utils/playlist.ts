import type { MediaFit, PlaylistConfig, PlaylistItem, ReadoutSlot, TemperatureOverlay } from "@/types";
import { playableKind } from "@/utils/mediaPicker";

/** Limits and defaults mirror `lianli_shared::playlist`. */
export const PLAYLIST_MAX_ITEMS = 200;
export const PLAYLIST_MAX_FOLDERS = 8;
export const PLAYLIST_MAX_SECONDS = 3600;
export const PLAYLIST_MAX_PLAYS = 100;
export const PLAYLIST_MAX_PROCESSES = 16;
export const READOUT_LABEL_MAX = 12;
export const CROSSFADE_MS = { min: 100, max: 3000, default: 600 };
export const RESERVE_PX = { min: 64, max: 400, default: 120 };

export function defaultOverlay(): TemperatureOverlay {
  return {
    fahrenheit: false,
    colors: true,
    warm_c: 75,
    hot_c: 85,
    reserve_px: 0,
  };
}

export function defaultPlaylist(): PlaylistConfig {
  return {
    items: [],
    image_seconds: 10,
    min_animation_seconds: 0,
    max_animation_seconds: 0,
    shuffle: false,
    transition: { crossfade: false, crossfade_ms: CROSSFADE_MS.default },
    overlay: defaultOverlay(),
    alternate: null,
    dimming: null,
  };
}

/** Mirrors `TemperatureOverlay::effective_slots`. */
export function effectiveSlots(overlay: TemperatureOverlay): [ReadoutSlot, ReadoutSlot] {
  if (overlay.slots) return overlay.slots;
  return [
    { kind: overlay.cpu ? "cpu_temp" : "off", source: overlay.cpu_source, sparkline: false },
    { kind: overlay.gpu ? "gpu_temp" : "off", source: overlay.gpu_source, sparkline: false },
  ];
}

/** Moves an older CPU/GPU configuration into editable slots. */
export function ensureSlots(overlay: TemperatureOverlay): [ReadoutSlot, ReadoutSlot] {
  if (!overlay.slots) {
    overlay.slots = effectiveSlots(overlay).map((slot) => {
      const copy: ReadoutSlot = { kind: slot.kind, sparkline: slot.sparkline };
      if (slot.source) copy.source = slot.source;
      return copy;
    }) as [ReadoutSlot, ReadoutSlot];
    delete overlay.cpu;
    delete overlay.gpu;
    delete overlay.cpu_source;
    delete overlay.gpu_source;
  }
  return overlay.slots;
}

export function overlayEnabled(overlay: TemperatureOverlay | undefined): boolean {
  return !!overlay && effectiveSlots(overlay).some((slot) => slot.kind !== "off");
}

/** Appends playable paths not already present, up to the item limit. */
export function appendItems(items: PlaylistItem[], paths: string[], limit = PLAYLIST_MAX_ITEMS): number {
  const present = new Set(items.map((item) => item.path));
  let added = 0;
  for (const path of paths) {
    if (items.length >= limit) break;
    if (present.has(path) || playableKind(path) === null) continue;
    present.add(path);
    items.push({ path });
    added++;
  }
  return added;
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

export function itemCount(playlist: PlaylistConfig): number {
  return playlist.items.length + (playlist.alternate?.items.length ?? 0);
}

export function isTime(text: string): boolean {
  const match = /^(\d{1,2}):(\d{2})$/.exec(text);
  return !!match && Number(match[1]) < 24 && Number(match[2]) < 60;
}

export interface Area {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Narrowest bar that shows a readout, as in lianli-media's playlist renderer. */
export const READOUT_MIN_BAR = 64;

/** Mirrors `place` for Fit mode in lianli-media. */
export function fitArea(source: [number, number], canvas: [number, number]): Area {
  const [cw, ch] = [Math.max(1, canvas[0]), Math.max(1, canvas[1])];
  const [sw, sh] = [Math.max(1, source[0]), Math.max(1, source[1])];
  const scale = Math.min(cw / sw, ch / sh);
  const width = Math.min(cw, Math.max(1, Math.round(sw * scale)));
  const height = Math.min(ch, Math.max(1, Math.round(sh * scale)));
  return { x: Math.floor((cw - width) / 2), y: Math.floor((ch - height) / 2), width, height };
}

/** Mirrors `side_bars` in lianli-media: first bar left or top, second right or bottom. */
export function readoutBars(media: Area, canvas: [number, number]): [Area, Area] | null {
  const [cw, ch] = canvas;
  const right = Math.max(0, cw - (media.x + media.width));
  const bottom = Math.max(0, ch - (media.y + media.height));
  const beside = Math.min(media.x, right);
  const around = Math.min(media.y, bottom);
  if (beside >= READOUT_MIN_BAR && beside >= around) {
    return [
      { x: 0, y: 0, width: media.x, height: ch },
      { x: media.x + media.width, y: 0, width: right, height: ch },
    ];
  }
  if (around >= READOUT_MIN_BAR) {
    return [
      { x: 0, y: 0, width: cw, height: media.y },
      { x: 0, y: media.y + media.height, width: cw, height: bottom },
    ];
  }
  return null;
}

/** Mirrors `place_reserving` in lianli-media. */
export function placeReserving(source: [number, number], canvas: [number, number], fit: MediaFit, reserve: number | null): Area {
  const full = { x: 0, y: 0, width: canvas[0], height: canvas[1] };
  if (fit !== "contain") return full;
  const natural = fitArea(source, canvas);
  if (!reserve) return natural;
  const minimum = Math.max(reserve, READOUT_MIN_BAR);
  const bars = readoutBars(natural, canvas);
  if (bars && Math.min(bars[0].width, bars[0].height) >= minimum) return natural;
  const [cw, ch] = canvas;
  const shrink = (inner: [number, number], offset: [number, number]): Area | null => {
    if (inner[0] <= 0 || inner[1] <= 0) return null;
    const area = fitArea(source, inner);
    return { ...area, x: area.x + offset[0], y: area.y + offset[1] };
  };
  const beside = shrink([cw - 2 * minimum, ch], [minimum, 0]);
  const around = shrink([cw, ch - 2 * minimum], [0, minimum]);
  const size = (area: Area) => area.width * area.height;
  if (beside && around) return size(beside) >= size(around) ? beside : around;
  return beside ?? around ?? natural;
}

/** Mirrors `halves` in lianli-media, for readouts without media. */
export function halves(canvas: [number, number]): [Area, Area] {
  const [cw, ch] = canvas;
  if (ch >= cw) {
    const top = Math.floor(ch / 2);
    return [{ x: 0, y: 0, width: cw, height: top }, { x: 0, y: top, width: cw, height: ch - top }];
  }
  const left = Math.floor(cw / 2);
  return [{ x: 0, y: 0, width: left, height: ch }, { x: left, y: 0, width: cw - left, height: ch }];
}

import type { PlaylistConfig } from "@/types";
import { playableKind } from "@/utils/mediaPicker";

/** Limits mirror `lianli_shared::playlist`. */
export const PLAYLIST_MAX_ITEMS = 500;
export const PLAYLIST_MAX_SECONDS = 3600;

export function defaultPlaylist(): PlaylistConfig {
  return {
    items: [],
    image_seconds: 10,
    min_animation_seconds: 0,
    max_animation_seconds: 0,
    shuffle: false,
    overlay: { cpu: false, gpu: false },
  };
}

/** Appends playable paths not already present, up to the item limit. */
export function appendItems(playlist: PlaylistConfig, paths: string[]): { added: number; skipped: number } {
  const present = new Set(playlist.items);
  let added = 0;
  for (const path of paths) {
    if (playlist.items.length >= PLAYLIST_MAX_ITEMS) break;
    if (present.has(path) || playableKind(path) === null) continue;
    present.add(path);
    playlist.items.push(path);
    added++;
  }
  return { added, skipped: paths.length - added };
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
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

/** Mirrors `side_bars` in lianli-media: CPU bar first, GPU bar second. */
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

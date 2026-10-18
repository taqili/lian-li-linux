import { open } from "@tauri-apps/plugin-dialog";
import { tr, type MessageKey } from "@/i18n";

const videoExtensions = ["mp4", "webm", "mkv", "mov", "avi", "m4v"];
const filters = {
  image: { name: "lcd.picker.images", invalid: "lcd.picker.chooseImages", extensions: ["png", "jpg", "jpeg", "bmp"] },
  video: { name: "lcd.picker.videos", invalid: "lcd.picker.chooseVideos", extensions: videoExtensions },
  gif: { name: "lcd.picker.gifAnimations", invalid: "lcd.picker.chooseGifAnimations", extensions: ["gif"] },
  animation: { name: "lcd.picker.animations", invalid: "lcd.picker.chooseAnimations", extensions: [...videoExtensions, "gif", "apng", "png"] },
} satisfies Record<string, { name: MessageKey; invalid: MessageKey; extensions: string[] }>;

export type MediaFileKind = keyof typeof filters;

export type PlayableKind = "image" | "gif" | "video";

/** Mirrors `PlaylistItemKind::from_path` in lianli-shared. */
export function playableKind(path: string): PlayableKind | null {
  return (["image", "gif", "video"] as const).find((kind) => matchesMediaFile(path, kind)) ?? null;
}

export async function pickPlayableFiles(): Promise<string[]> {
  const extensions = [...filters.image.extensions, ...filters.gif.extensions, ...filters.video.extensions];
  const selected = await open({
    multiple: true,
    directory: false,
    filters: [{ name: tr("lcd.picker.playlistMedia"), extensions }],
  });
  const paths = Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
  return paths.filter((path) => playableKind(path) !== null);
}

export async function pickFolder(): Promise<string | null> {
  const selected = await open({ multiple: false, directory: true });
  return typeof selected === "string" ? selected : null;
}

export function matchesMediaFile(path: string, kind: MediaFileKind): boolean {
  const extension = /\.([^.\\/]+)$/.exec(path)?.[1].toLowerCase() ?? "";
  return filters[kind].extensions.includes(extension);
}

export async function pickMediaFile(kind: MediaFileKind): Promise<string | null> {
  const filter = filters[kind];
  const selected = await open({ multiple: false, directory: false, filters: [{ name: tr(filter.name), extensions: filter.extensions }] });
  if (typeof selected !== "string") return null;
  if (!matchesMediaFile(selected, kind)) {
    throw new Error(tr(filter.invalid, { extensions: filter.extensions.map(extension => `.${extension}`).join(", ") }));
  }
  return selected;
}

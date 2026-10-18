<script setup lang="ts">
import { computed, ref } from "vue";
import { useMessage } from "naive-ui";
import { ArrowDown, ArrowUp, FilePlus, FolderPlus, SlidersHorizontal, X } from "lucide-vue-next";
import type { MediaFit, PlaylistFolder, PlaylistItem } from "@/types";
import { useConfigStore } from "@/stores/config";
import { invoke } from "@/utils/backendText";
import { pickFolder, pickPlayableFiles, playableKind, type PlayableKind } from "@/utils/mediaPicker";
import { appendItems, fileName, PLAYLIST_MAX_FOLDERS, PLAYLIST_MAX_ITEMS, PLAYLIST_MAX_PLAYS, PLAYLIST_MAX_SECONDS } from "@/utils/playlist";
import { useTranslate, type MessageKey } from "@/i18n";

const props = defineProps<{
  items: PlaylistItem[];
  /** Remaining room across every list in the playlist. */
  room: number;
  selected?: number;
  /** Owner of the live folders; omitted where folders are not offered. */
  folderHost?: { folders?: PlaylistFolder[] };
}>();
const folders = computed(() => props.folderHost?.folders ?? []);
const emit = defineEmits<{ "update:selected": [index: number] }>();

const config = useConfigStore();
const message = useMessage();
const t = useTranslate();

const KIND_LABELS: Record<PlayableKind, MessageKey> = {
  image: "lcd.mediaTypes.image",
  gif: "lcd.mediaTypes.gif",
  video: "lcd.mediaTypes.video",
};
const FIT_LABELS: Record<MediaFit, MessageKey> = {
  stretch: "lcd.configCard.fitOptions.stretch",
  contain: "lcd.configCard.fitOptions.contain",
  cover: "lcd.configCard.fitOptions.cover",
};

const recursive = ref(false);
const live = ref(true);
const scanning = ref(false);
const expanded = ref<string | null>(null);
const full = computed(() => props.room <= 0);
const fitOptions = computed(() => [
  { label: t("lcd.playlist.useDefault"), value: "" },
  ...(Object.keys(FIT_LABELS) as MediaFit[]).map((fit) => ({ label: t(FIT_LABELS[fit]), value: fit })),
]);

function changed() {
  config.markDirty();
}

function report(added: number, offered: number) {
  if (added > 0) message.success(t("lcd.playlist.added", { count: added }));
  if (props.room <= 0 && added < offered) message.warning(t("lcd.playlist.limitReached", { max: PLAYLIST_MAX_ITEMS }));
  else if (added === 0) message.info(t("lcd.playlist.nothingAdded"));
}

async function addFiles() {
  try {
    const paths = await pickPlayableFiles();
    if (!paths.length) return;
    const added = appendItems(props.items, paths, props.items.length + props.room);
    if (added) changed();
    report(added, paths.length);
  } catch (error) {
    message.error(String(error));
  }
}

async function addFolder() {
  if (scanning.value) return;
  try {
    const folder = await pickFolder();
    if (!folder) return;
    if (props.folderHost && live.value) {
      if (folders.value.some((existing) => existing.path === folder)) return;
      if (folders.value.length >= PLAYLIST_MAX_FOLDERS) {
        message.warning(t("lcd.playlist.folderLimit", { max: PLAYLIST_MAX_FOLDERS }));
        return;
      }
      (props.folderHost.folders ??= []).push({ path: folder, recursive: recursive.value });
      changed();
      return;
    }
    scanning.value = true;
    const result = await invoke<{ items: string[]; truncated: boolean }>("list_media_folder", {
      path: folder,
      recursive: recursive.value,
    });
    const added = appendItems(props.items, result.items, props.items.length + props.room);
    if (added) changed();
    if (result.truncated) message.warning(t("lcd.playlist.folderTruncated", { max: PLAYLIST_MAX_ITEMS }));
    report(added, result.items.length);
  } catch (error) {
    message.error(String(error));
  } finally {
    scanning.value = false;
  }
}

function move(index: number, delta: number) {
  const target = index + delta;
  if (target < 0 || target >= props.items.length) return;
  [props.items[index], props.items[target]] = [props.items[target], props.items[index]];
  if (props.selected === index) emit("update:selected", target);
  else if (props.selected === target) emit("update:selected", index);
  changed();
}

function remove(index: number) {
  props.items.splice(index, 1);
  const selected = props.selected ?? 0;
  if (selected >= props.items.length) emit("update:selected", Math.max(0, props.items.length - 1));
  else if (selected > index) emit("update:selected", selected - 1);
  changed();
}

function clearAll() {
  props.items.splice(0);
  emit("update:selected", 0);
  changed();
}

type NumberField = "seconds" | "plays" | "start_seconds" | "end_seconds";
function setNumber(item: PlaylistItem, field: NumberField, value: number | null) {
  if (value === null || !Number.isFinite(value) || (field !== "start_seconds" && value <= 0)) delete item[field];
  else item[field] = field === "plays" || field === "seconds" ? Math.round(value) : value;
  changed();
}

function setFit(item: PlaylistItem, fit: string) {
  if (fit) item.fit = fit as MediaFit;
  else delete item.fit;
  changed();
}

function overridden(item: PlaylistItem): boolean {
  return item.seconds !== undefined || item.plays !== undefined || item.fit !== undefined
    || item.start_seconds !== undefined || item.end_seconds !== undefined;
}

function trimError(item: PlaylistItem): boolean {
  return item.end_seconds !== undefined && item.end_seconds <= (item.start_seconds ?? 0);
}

function removeFolder(index: number) {
  const host = props.folderHost;
  host?.folders?.splice(index, 1);
  if (host && !host.folders?.length) delete host.folders;
  changed();
}
</script>

<template>
  <div class="item-list">
    <div class="list-actions">
      <n-button size="small" :disabled="full" @click="addFiles">
        <template #icon><FilePlus :size="14" /></template>
        {{ t("lcd.playlist.addFiles") }}
      </n-button>
      <n-button size="small" :disabled="full && !(folderHost && live)" :loading="scanning" @click="addFolder">
        <template #icon><FolderPlus :size="14" /></template>
        {{ t("lcd.playlist.addFolder") }}
      </n-button>
      <n-checkbox v-if="folderHost" :checked="live" @update:checked="(value: boolean) => (live = value)">
        {{ t("lcd.playlist.keepInSync") }}
      </n-checkbox>
      <n-checkbox :checked="recursive" @update:checked="(value: boolean) => (recursive = value)">
        {{ t("lcd.playlist.includeSubfolders") }}
      </n-checkbox>
      <n-button size="small" quaternary :disabled="!items.length" @click="clearAll">{{ t("lcd.playlist.clear") }}</n-button>
    </div>

    <ul v-if="folders?.length" class="folders">
      <li v-for="(folder, index) in folders" :key="folder.path">
        <span class="kind">{{ t("lcd.playlist.liveFolder") }}</span>
        <span class="name" :title="folder.path">{{ folder.path }}</span>
        <n-checkbox size="small" :checked="folder.recursive" @update:checked="(value: boolean) => { folder.recursive = value; changed(); }">
          {{ t("lcd.playlist.subfolders") }}
        </n-checkbox>
        <n-button size="tiny" quaternary :title="t('common.remove')" @click="removeFolder(index)">
          <template #icon><X :size="12" /></template>
        </n-button>
      </li>
    </ul>

    <ol v-if="items.length" class="items">
      <li v-for="(item, index) in items" :key="`${index}:${item.path}`" :class="{ selected: index === selected }">
        <div class="row" @click="emit('update:selected', index)">
          <span class="kind">{{ playableKind(item.path) ? t(KIND_LABELS[playableKind(item.path)!]) : "?" }}</span>
          <span class="name" :title="item.path">{{ fileName(item.path) }}</span>
          <span class="row-actions" @click.stop>
            <n-button
              size="tiny"
              quaternary
              :type="overridden(item) ? 'primary' : 'default'"
              :title="t('lcd.playlist.itemOptions')"
              @click="expanded = expanded === item.path ? null : item.path"
            >
              <template #icon><SlidersHorizontal :size="12" /></template>
            </n-button>
            <n-button size="tiny" quaternary :disabled="index === 0" :title="t('lcd.playlist.moveUp')" @click="move(index, -1)">
              <template #icon><ArrowUp :size="12" /></template>
            </n-button>
            <n-button size="tiny" quaternary :disabled="index === items.length - 1" :title="t('lcd.playlist.moveDown')" @click="move(index, 1)">
              <template #icon><ArrowDown :size="12" /></template>
            </n-button>
            <n-button size="tiny" quaternary :title="t('common.remove')" @click="remove(index)">
              <template #icon><X :size="12" /></template>
            </n-button>
          </span>
        </div>
        <div v-if="expanded === item.path" class="overrides">
          <div v-if="playableKind(item.path) === 'image'" class="field">
            <label class="muted">{{ t("lcd.playlist.itemSeconds") }}</label>
            <n-input-number
              :value="item.seconds ?? null"
              size="small"
              clearable
              :min="1"
              :max="PLAYLIST_MAX_SECONDS"
              :placeholder="t('lcd.playlist.useDefault')"
              @update:value="(v: number | null) => setNumber(item, 'seconds', v)"
            />
          </div>
          <div v-else class="field">
            <label class="muted">{{ t("lcd.playlist.itemPlays") }}</label>
            <n-input-number
              :value="item.plays ?? null"
              size="small"
              clearable
              :min="1"
              :max="PLAYLIST_MAX_PLAYS"
              :placeholder="t('lcd.playlist.useDefault')"
              @update:value="(v: number | null) => setNumber(item, 'plays', v)"
            />
          </div>
          <template v-if="playableKind(item.path) === 'video'">
            <div class="field">
              <label class="muted">{{ t("lcd.playlist.trimStart") }}</label>
              <n-input-number :value="item.start_seconds ?? null" size="small" clearable :min="0" :step="0.5" @update:value="(v: number | null) => setNumber(item, 'start_seconds', v)" />
            </div>
            <div class="field">
              <label class="muted">{{ t("lcd.playlist.trimEnd") }}</label>
              <n-input-number :value="item.end_seconds ?? null" size="small" clearable :min="0" :step="0.5" @update:value="(v: number | null) => setNumber(item, 'end_seconds', v)" />
            </div>
          </template>
          <div class="field">
            <label class="muted">{{ t("lcd.configCard.fit") }}</label>
            <n-select :value="item.fit ?? ''" :options="fitOptions" size="small" @update:value="(v: string) => setFit(item, v)" />
          </div>
          <p v-if="trimError(item)" class="error">{{ t("lcd.playlist.trimOrder") }}</p>
        </div>
      </li>
    </ol>
  </div>
</template>

<style scoped>
.item-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.list-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}
.folders,
.items {
  list-style: none;
  margin: 0;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.items {
  max-height: 260px;
  overflow-y: auto;
}
.folders li,
.row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 2px var(--space-2);
  font-size: var(--font-size-sm);
}
.folders li + li,
.items li + li {
  border-top: 1px solid var(--border);
}
.row {
  cursor: pointer;
}
.row:hover {
  background: var(--bg-hover);
}
.items li.selected > .row {
  background: var(--accent-soft);
}
.kind {
  flex-shrink: 0;
  min-width: 44px;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
.name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.row-actions {
  display: flex;
  flex-shrink: 0;
}
.overrides {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: var(--space-2);
  padding: var(--space-2);
  border-top: 1px dashed var(--border);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.error {
  grid-column: 1 / -1;
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--danger);
}
</style>

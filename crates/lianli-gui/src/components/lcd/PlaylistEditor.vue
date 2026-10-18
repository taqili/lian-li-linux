<script setup lang="ts">
import { computed, ref } from "vue";
import { useMessage } from "naive-ui";
import { ArrowDown, ArrowUp, FilePlus, FolderPlus, X } from "lucide-vue-next";
import type { MediaFit, PlaylistConfig, SensorSourceConfig } from "@/types";
import { useConfigStore } from "@/stores/config";
import { invoke } from "@/utils/backendText";
import { pickFolder, pickPlayableFiles, playableKind, type PlayableKind } from "@/utils/mediaPicker";
import { appendItems, fileName, PLAYLIST_MAX_ITEMS, PLAYLIST_MAX_SECONDS } from "@/utils/playlist";
import { decodeOption, sourceConfigsAsOptions } from "@/stores/sensorOptions";
import SensorSelect from "@/components/common/SensorSelect.vue";
import { useTranslate, type MessageKey } from "@/i18n";

const props = defineProps<{ playlist: PlaylistConfig; fit: MediaFit; selected: number }>();
const emit = defineEmits<{ "update:selected": [index: number] }>();

const config = useConfigStore();
const message = useMessage();
const t = useTranslate();

const KIND_LABELS: Record<PlayableKind, MessageKey> = {
  image: "lcd.mediaTypes.image",
  gif: "lcd.mediaTypes.gif",
  video: "lcd.mediaTypes.video",
};

const recursive = ref(false);
const scanning = ref(false);
const full = computed(() => props.playlist.items.length >= PLAYLIST_MAX_ITEMS);

function changed() {
  config.markDirty();
}

function report(added: number, offered: number) {
  if (added > 0) message.success(t("lcd.playlist.added", { count: added }));
  if (props.playlist.items.length >= PLAYLIST_MAX_ITEMS && added < offered) {
    message.warning(t("lcd.playlist.limitReached", { max: PLAYLIST_MAX_ITEMS }));
  } else if (added === 0) {
    message.info(t("lcd.playlist.nothingAdded"));
  }
}

async function addFiles() {
  try {
    const paths = await pickPlayableFiles();
    if (!paths.length) return;
    const { added } = appendItems(props.playlist, paths);
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
    scanning.value = true;
    const result = await invoke<{ items: string[]; truncated: boolean }>("list_media_folder", {
      path: folder,
      recursive: recursive.value,
    });
    const { added } = appendItems(props.playlist, result.items);
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
  const items = props.playlist.items;
  if (target < 0 || target >= items.length) return;
  [items[index], items[target]] = [items[target], items[index]];
  if (props.selected === index) emit("update:selected", target);
  else if (props.selected === target) emit("update:selected", index);
  changed();
}

function remove(index: number) {
  props.playlist.items.splice(index, 1);
  if (props.selected >= props.playlist.items.length) emit("update:selected", Math.max(0, props.playlist.items.length - 1));
  else if (props.selected > index) emit("update:selected", props.selected - 1);
  changed();
}

function clearAll() {
  props.playlist.items.splice(0);
  emit("update:selected", 0);
  changed();
}

function setSeconds(key: "image_seconds" | "min_animation_seconds" | "max_animation_seconds", value: number | null) {
  const floor = key === "image_seconds" ? 1 : 0;
  props.playlist[key] = Math.min(PLAYLIST_MAX_SECONDS, Math.max(floor, Math.round(value ?? floor)));
  changed();
}

const durationError = computed(() => {
  const { min_animation_seconds: min, max_animation_seconds: max } = props.playlist;
  return max > 0 && max < min ? t("lcd.playlist.maxBelowMin") : "";
});

const AUTO = "auto";
const temperatureOptions = computed(() => [
  { label: t("lcd.playlist.automaticSensor"), value: AUTO },
  ...sourceConfigsAsOptions(config.sensors.filter((sensor) => sensor.unit === "C")),
]);

function sourceValue(source: SensorSourceConfig | undefined): string {
  return source ? JSON.stringify(source) : AUTO;
}

function setSource(key: "cpu_source" | "gpu_source", value: string) {
  const source = value === AUTO ? null : decodeOption(value);
  if (source) props.playlist.overlay[key] = source;
  else delete props.playlist.overlay[key];
  changed();
}

function setShown(key: "cpu" | "gpu", shown: boolean) {
  props.playlist.overlay[key] = shown;
  changed();
}

const overlayEnabled = computed(() => props.playlist.overlay.cpu || props.playlist.overlay.gpu);
</script>

<template>
  <div class="playlist">
    <div class="list-head">
      <label class="muted">{{ t("lcd.playlist.items", { count: playlist.items.length, max: PLAYLIST_MAX_ITEMS }) }}</label>
      <div class="list-actions">
        <n-button size="small" :disabled="full" @click="addFiles">
          <template #icon><FilePlus :size="14" /></template>
          {{ t("lcd.playlist.addFiles") }}
        </n-button>
        <n-button size="small" :disabled="full" :loading="scanning" @click="addFolder">
          <template #icon><FolderPlus :size="14" /></template>
          {{ t("lcd.playlist.addFolder") }}
        </n-button>
        <n-checkbox :checked="recursive" @update:checked="(value: boolean) => (recursive = value)">
          {{ t("lcd.playlist.includeSubfolders") }}
        </n-checkbox>
        <n-button size="small" quaternary :disabled="!playlist.items.length" @click="clearAll">{{ t("lcd.playlist.clear") }}</n-button>
      </div>
    </div>

    <p v-if="!playlist.items.length" class="hint">{{ t("lcd.playlist.empty") }}</p>
    <ol v-else class="items">
      <li
        v-for="(path, index) in playlist.items"
        :key="`${index}:${path}`"
        :class="{ selected: index === selected }"
        @click="emit('update:selected', index)"
      >
        <span class="kind">{{ playableKind(path) ? t(KIND_LABELS[playableKind(path)!]) : "?" }}</span>
        <span class="name" :title="path">{{ fileName(path) }}</span>
        <span class="row-actions" @click.stop>
          <n-button size="tiny" quaternary :disabled="index === 0" :title="t('lcd.playlist.moveUp')" @click="move(index, -1)">
            <template #icon><ArrowUp :size="12" /></template>
          </n-button>
          <n-button size="tiny" quaternary :disabled="index === playlist.items.length - 1" :title="t('lcd.playlist.moveDown')" @click="move(index, 1)">
            <template #icon><ArrowDown :size="12" /></template>
          </n-button>
          <n-button size="tiny" quaternary :title="t('common.remove')" @click="remove(index)">
            <template #icon><X :size="12" /></template>
          </n-button>
        </span>
      </li>
    </ol>
    <p v-if="playlist.items.length" class="hint">{{ t("lcd.playlist.folderSnapshot") }}</p>

    <div class="timing">
      <div class="field">
        <label class="muted">{{ t("lcd.playlist.imageSeconds") }}</label>
        <n-input-number :value="playlist.image_seconds" size="small" :min="1" :max="PLAYLIST_MAX_SECONDS" @update:value="(v: number | null) => setSeconds('image_seconds', v)" />
      </div>
      <div class="field">
        <label class="muted">{{ t("lcd.playlist.minAnimationSeconds") }}</label>
        <n-input-number :value="playlist.min_animation_seconds" size="small" :min="0" :max="PLAYLIST_MAX_SECONDS" @update:value="(v: number | null) => setSeconds('min_animation_seconds', v)" />
      </div>
      <div class="field">
        <label class="muted">{{ t("lcd.playlist.maxAnimationSeconds") }}</label>
        <n-input-number :value="playlist.max_animation_seconds" size="small" :min="0" :max="PLAYLIST_MAX_SECONDS" @update:value="(v: number | null) => setSeconds('max_animation_seconds', v)" />
      </div>
    </div>
    <p class="hint">{{ t("lcd.playlist.timingHint") }}</p>
    <n-alert v-if="durationError" type="error" :show-icon="false">{{ durationError }}</n-alert>
    <n-checkbox :checked="playlist.shuffle" @update:checked="(value: boolean) => { playlist.shuffle = value; changed(); }">
      {{ t("lcd.playlist.shuffle") }}
    </n-checkbox>

    <div class="divider" />

    <label class="muted">{{ t("lcd.playlist.temperatures") }}</label>
    <div class="readouts">
      <div class="readout">
        <n-checkbox :checked="playlist.overlay.cpu" @update:checked="(value: boolean) => setShown('cpu', value)">
          {{ t("lcd.playlist.showCpu") }}
        </n-checkbox>
        <SensorSelect
          v-if="playlist.overlay.cpu"
          :value="sourceValue(playlist.overlay.cpu_source)"
          :options="temperatureOptions"
          size="small"
          @update:value="(value: string) => setSource('cpu_source', value)"
        />
      </div>
      <div class="readout">
        <n-checkbox :checked="playlist.overlay.gpu" @update:checked="(value: boolean) => setShown('gpu', value)">
          {{ t("lcd.playlist.showGpu") }}
        </n-checkbox>
        <SensorSelect
          v-if="playlist.overlay.gpu"
          :value="sourceValue(playlist.overlay.gpu_source)"
          :options="temperatureOptions"
          size="small"
          @update:value="(value: string) => setSource('gpu_source', value)"
        />
      </div>
    </div>
    <p v-if="overlayEnabled" class="hint">{{ t("lcd.playlist.temperatureHint") }}</p>
    <n-alert v-if="overlayEnabled && fit !== 'contain'" type="warning" :show-icon="false">
      {{ t("lcd.playlist.needsFit") }}
    </n-alert>
  </div>
</template>

<style scoped>
.playlist {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.list-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}
.list-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}
.items {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 240px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.items li {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 2px var(--space-2);
  cursor: pointer;
  font-size: var(--font-size-sm);
}
.items li + li {
  border-top: 1px solid var(--border);
}
.items li:hover {
  background: var(--bg-hover);
}
.items li.selected {
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
.timing {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: var(--space-2) var(--space-3);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.readouts {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: var(--space-2) var(--space-3);
}
.readout {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.divider {
  border-top: 1px solid var(--border);
}
.hint {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
</style>

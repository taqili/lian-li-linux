<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { useMessage } from "naive-ui";
import { Pause, Play, SkipBack, SkipForward } from "lucide-vue-next";
import type { MediaFit, PlaylistAction, PlaylistConfig, PlaylistStatus, ReadoutPreview } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useIpc } from "@/composables/useIpc";
import { CROSSFADE_MS, fileName, itemCount, PLAYLIST_MAX_ITEMS, PLAYLIST_MAX_SECONDS } from "@/utils/playlist";
import PlaylistItemList from "@/components/lcd/PlaylistItemList.vue";
import PlaylistReadouts from "@/components/lcd/PlaylistReadouts.vue";
import PlaylistSchedule from "@/components/lcd/PlaylistSchedule.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import { useTranslate } from "@/i18n";

const props = defineProps<{
  playlist: PlaylistConfig;
  fit: MediaFit;
  selected: number;
  previews: ReadoutPreview[] | null;
  /** Screen to control; transport controls hide when it cannot be reached. */
  deviceId?: string;
  canControl: boolean;
}>();
const emit = defineEmits<{ "update:selected": [index: number] }>();

const config = useConfigStore();
const ipc = useIpc();
const message = useMessage();
const t = useTranslate();


function changed() {
  config.markDirty();
}

const room = computed(() => PLAYLIST_MAX_ITEMS - itemCount(props.playlist));

function setSeconds(key: "image_seconds" | "min_animation_seconds" | "max_animation_seconds", value: number | null) {
  const floor = key === "image_seconds" ? 1 : 0;
  props.playlist[key] = Math.min(PLAYLIST_MAX_SECONDS, Math.max(floor, Math.round(value ?? floor)));
  changed();
}

const durationError = computed(() => {
  const { min_animation_seconds: min, max_animation_seconds: max } = props.playlist;
  return max > 0 && max < min ? t("lcd.playlist.maxBelowMin") : "";
});

const status = ref<PlaylistStatus | null>(null);
const busy = ref(false);

async function control(action: PlaylistAction, quiet = false) {
  if (!props.deviceId || busy.value) return;
  busy.value = !quiet;
  try {
    const replies = await ipc.request<{ device_id: string; status: PlaylistStatus }[]>("PlaylistControl", {
      device_id: props.deviceId,
      action,
    });
    status.value = replies[0]?.status ?? null;
  } catch (error) {
    status.value = null;
    if (!quiet) message.error(String(error));
  } finally {
    busy.value = false;
  }
}

let statusTimer: ReturnType<typeof setInterval> | undefined;
watch(
  () => [props.canControl, props.deviceId] as const,
  ([enabled]) => {
    clearInterval(statusTimer);
    status.value = null;
    if (!enabled) return;
    void control("status", true);
    statusTimer = setInterval(() => {
      if (!document.hidden) void control("status", true);
    }, 5000);
  },
  { immediate: true },
);
onUnmounted(() => clearInterval(statusTimer));

const nowPlaying = computed(() => {
  if (!status.value) return "";
  if (!status.value.current) return t("lcd.playlist.showingReadouts");
  const item = fileName(status.value.current);
  if (status.value.paused) return t("lcd.playlist.pausedOn", { item });
  return status.value.alternate ? t("lcd.playlist.playingAlternate", { item }) : t("lcd.playlist.playing", { item });
});
</script>

<template>
  <div class="playlist">
    <div v-if="canControl && deviceId" class="transport">
      <n-button size="small" :disabled="busy" :title="t('lcd.playlist.previous')" @click="control('previous')">
        <template #icon><SkipBack :size="14" /></template>
      </n-button>
      <n-button size="small" :disabled="busy" :title="status?.paused ? t('lcd.playlist.resume') : t('lcd.playlist.pause')" @click="control('toggle_pause')">
        <template #icon><Play v-if="status?.paused" :size="14" /><Pause v-else :size="14" /></template>
      </n-button>
      <n-button size="small" :disabled="busy" :title="t('lcd.playlist.next')" @click="control('next')">
        <template #icon><SkipForward :size="14" /></template>
      </n-button>
      <span class="now" :title="status?.current ?? ''">{{ nowPlaying }}</span>
    </div>
    <p v-else class="hint">{{ t("lcd.playlist.saveForControls") }}</p>

    <label class="muted">{{ t("lcd.playlist.items", { count: itemCount(playlist), max: PLAYLIST_MAX_ITEMS }) }}</label>
    <PlaylistItemList
      :items="playlist.items"
      :folder-host="playlist"
      :room="room"
      :selected="selected"
      @update:selected="(index: number) => emit('update:selected', index)"
    />
    <p v-if="!playlist.items.length && !playlist.folders?.length" class="hint">{{ t("lcd.playlist.empty") }}</p>
    <p v-else-if="playlist.folders?.length" class="hint">{{ t("lcd.playlist.liveHint") }}</p>
    <p v-else class="hint">{{ t("lcd.playlist.folderSnapshot") }}</p>

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

    <div class="toggles">
      <n-checkbox :checked="playlist.shuffle" @update:checked="(value: boolean) => { playlist.shuffle = value; changed(); }">
        {{ t("lcd.playlist.shuffle") }}
      </n-checkbox>
      <div class="crossfade">
        <n-switch
          :value="playlist.transition.crossfade"
          size="small"
          @update:value="(value: boolean) => { playlist.transition.crossfade = value; changed(); }"
        />
        <span>{{ t("lcd.playlist.crossfade") }}</span>
      </div>
    </div>
    <div v-if="playlist.transition.crossfade" class="field">
      <label class="muted">{{ t("lcd.playlist.crossfadeLength") }}</label>
      <LabeledSlider
        :model-value="playlist.transition.crossfade_ms"
        :min="CROSSFADE_MS.min"
        :max="CROSSFADE_MS.max"
        :step="100"
        suffix=" ms"
        @update:model-value="(v: number) => { playlist.transition.crossfade_ms = v; changed(); }"
      />
    </div>

    <n-collapse class="sections" :default-expanded-names="['readouts']">
      <n-collapse-item :title="t('lcd.readouts.title')" name="readouts">
        <PlaylistReadouts :overlay="playlist.overlay" :fit="fit" :previews="previews" />
      </n-collapse-item>
      <n-collapse-item :title="t('lcd.schedule.title')" name="schedule">
        <PlaylistSchedule :playlist="playlist" />
      </n-collapse-item>
    </n-collapse>
  </div>
</template>

<style scoped>
.playlist {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.transport {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}
.now {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
.timing {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: var(--space-2) var(--space-3);
}
.toggles {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
}
.crossfade {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.hint {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
</style>

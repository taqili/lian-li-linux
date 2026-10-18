<script setup lang="ts">
import { computed } from "vue";
import type { PlaylistConfig, TimeWindow } from "@/types";
import { useConfigStore } from "@/stores/config";
import { isTime, itemCount, overlayEnabled, PLAYLIST_MAX_ITEMS, PLAYLIST_MAX_PROCESSES } from "@/utils/playlist";
import PlaylistItemList from "@/components/lcd/PlaylistItemList.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import { useTranslate } from "@/i18n";

const props = defineProps<{ playlist: PlaylistConfig }>();

const config = useConfigStore();
const t = useTranslate();

function changed() {
  config.markDirty();
}

const alternate = computed(() => props.playlist.alternate ?? null);
const dimming = computed(() => props.playlist.dimming ?? null);
const room = computed(() => PLAYLIST_MAX_ITEMS - itemCount(props.playlist));

function setAlternate(on: boolean) {
  props.playlist.alternate = on
    ? { items: [], readouts_only: overlayEnabled(props.playlist.overlay), processes: [], window: { start: "22:00", end: "07:00" } }
    : null;
  changed();
}

function setReadoutsOnly(readoutsOnly: boolean) {
  if (!alternate.value) return;
  alternate.value.readouts_only = readoutsOnly;
  changed();
}

function setProcesses(names: string[]) {
  if (!alternate.value) return;
  alternate.value.processes = names
    .map((name) => name.trim())
    .filter((name, index, all) => name && !name.includes("/") && name.length <= 64 && all.indexOf(name) === index)
    .slice(0, PLAYLIST_MAX_PROCESSES);
  changed();
}

function setWindowEnabled(on: boolean) {
  if (!alternate.value) return;
  if (on) alternate.value.window = { start: "22:00", end: "07:00" };
  else delete alternate.value.window;
  changed();
}

function setTime(window: TimeWindow, key: "start" | "end", value: string | null) {
  if (value && isTime(value)) {
    window[key] = value;
    changed();
  }
}

function setDimming(on: boolean) {
  props.playlist.dimming = on ? { window: { start: "23:00", end: "07:00" }, percent: 40 } : null;
  changed();
}

function windowError(window: TimeWindow | undefined): boolean {
  return !!window && window.start === window.end;
}

const alternateProblem = computed(() => {
  const value = alternate.value;
  if (!value) return "";
  if (!value.processes.length && !value.window) return t("lcd.schedule.needsTrigger");
  if (value.readouts_only && !overlayEnabled(props.playlist.overlay)) return t("lcd.schedule.needsReadout");
  if (!value.readouts_only && !value.items.length) return t("lcd.schedule.needsItems");
  return "";
});
</script>

<template>
  <div class="schedule">
    <n-checkbox :checked="!!alternate" @update:checked="setAlternate">{{ t("lcd.schedule.useAlternate") }}</n-checkbox>
    <template v-if="alternate">
      <p class="hint">{{ t("lcd.schedule.alternateHint") }}</p>
      <n-radio-group :value="alternate.readouts_only" size="small" @update:value="setReadoutsOnly">
        <n-radio-button :value="false">{{ t("lcd.schedule.playItems") }}</n-radio-button>
        <n-radio-button :value="true">{{ t("lcd.schedule.readoutsOnly") }}</n-radio-button>
      </n-radio-group>
      <PlaylistItemList v-if="!alternate.readouts_only" :items="alternate.items" :room="room" />
      <div class="field">
        <label class="muted">{{ t("lcd.schedule.programs") }}</label>
        <n-dynamic-tags
          :value="alternate.processes"
          :max="PLAYLIST_MAX_PROCESSES"
          @update:value="setProcesses"
        />
        <p class="hint">{{ t("lcd.schedule.programsHint") }}</p>
      </div>
      <n-checkbox :checked="!!alternate.window" @update:checked="setWindowEnabled">{{ t("lcd.schedule.duringHours") }}</n-checkbox>
      <div v-if="alternate.window" class="times">
        <n-time-picker
          :formatted-value="alternate.window.start"
          value-format="HH:mm"
          format="HH:mm"
          size="small"
          @update:formatted-value="(v: string | null) => setTime(alternate!.window!, 'start', v)"
        />
        <span class="muted">–</span>
        <n-time-picker
          :formatted-value="alternate.window.end"
          value-format="HH:mm"
          format="HH:mm"
          size="small"
          @update:formatted-value="(v: string | null) => setTime(alternate!.window!, 'end', v)"
        />
      </div>
      <p v-if="windowError(alternate.window)" class="error">{{ t("lcd.schedule.sameTimes") }}</p>
      <p v-if="alternateProblem" class="error">{{ alternateProblem }}</p>
    </template>

    <div class="divider" />

    <n-checkbox :checked="!!dimming" @update:checked="setDimming">{{ t("lcd.schedule.dim") }}</n-checkbox>
    <template v-if="dimming">
      <div class="times">
        <n-time-picker
          :formatted-value="dimming.window.start"
          value-format="HH:mm"
          format="HH:mm"
          size="small"
          @update:formatted-value="(v: string | null) => setTime(dimming!.window, 'start', v)"
        />
        <span class="muted">–</span>
        <n-time-picker
          :formatted-value="dimming.window.end"
          value-format="HH:mm"
          format="HH:mm"
          size="small"
          @update:formatted-value="(v: string | null) => setTime(dimming!.window, 'end', v)"
        />
      </div>
      <div class="field">
        <label class="muted">{{ t("lcd.schedule.dimTo") }}</label>
        <LabeledSlider
          :model-value="dimming.percent"
          :min="10"
          :max="100"
          suffix="%"
          @update:model-value="(v: number) => { dimming!.percent = v; changed(); }"
        />
      </div>
      <p class="hint">{{ t("lcd.schedule.dimHint") }}</p>
      <p v-if="windowError(dimming.window)" class="error">{{ t("lcd.schedule.sameTimes") }}</p>
    </template>
  </div>
</template>

<style scoped>
.schedule {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.times {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.divider {
  border-top: 1px solid var(--border);
}
.hint,
.error {
  margin: 0;
  font-size: var(--font-size-xs);
}
.hint {
  color: var(--text-muted);
}
.error {
  color: var(--danger);
}
</style>

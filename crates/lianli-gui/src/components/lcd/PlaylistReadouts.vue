<script setup lang="ts">
import { computed, onMounted } from "vue";
import type { MediaFit, ReadoutKind, ReadoutPreview, ReadoutSlot, TemperatureOverlay } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useFonts } from "@/composables/useFonts";
import { decodeOption, sourceConfigsAsOptions } from "@/stores/sensorOptions";
import { effectiveSlots, ensureSlots, overlayEnabled, READOUT_LABEL_MAX, RESERVE_PX } from "@/utils/playlist";
import SensorSelect from "@/components/common/SensorSelect.vue";
import { useTranslate, type MessageKey } from "@/i18n";

const props = defineProps<{ overlay: TemperatureOverlay; fit: MediaFit; previews: ReadoutPreview[] | null }>();

const config = useConfigStore();
const t = useTranslate();
const { load: loadFonts, fontOptions } = useFonts();
onMounted(() => void loadFonts());

const KIND_LABELS: Record<ReadoutKind, MessageKey> = {
  off: "lcd.readouts.kinds.off",
  cpu_temp: "lcd.readouts.kinds.cpuTemp",
  gpu_temp: "lcd.readouts.kinds.gpuTemp",
  gpu_hotspot: "lcd.readouts.kinds.gpuHotspot",
  cpu_usage: "lcd.readouts.kinds.cpuUsage",
  gpu_usage: "lcd.readouts.kinds.gpuUsage",
  clock: "lcd.readouts.kinds.clock",
  sensor: "lcd.readouts.kinds.sensor",
};
const POSITION_LABELS: [MessageKey, MessageKey] = ["lcd.readouts.first", "lcd.readouts.second"];
const AUTO = "auto";

const kindOptions = computed(() =>
  (Object.keys(KIND_LABELS) as ReadoutKind[]).map((kind) => ({ label: t(KIND_LABELS[kind]), value: kind })),
);
const slots = computed(() => effectiveSlots(props.overlay));
const temperatureSensors = computed(() => sourceConfigsAsOptions(config.sensors.filter((sensor) => sensor.unit === "C")));
const allSensors = computed(() => sourceConfigsAsOptions(config.sensors));

function sensorOptions(kind: ReadoutKind) {
  if (kind === "sensor") return allSensors.value;
  const automatic = { label: t("lcd.readouts.automatic"), value: AUTO };
  if (kind === "cpu_temp" || kind === "gpu_temp" || kind === "gpu_hotspot") return [automatic, ...temperatureSensors.value];
  return [automatic, ...allSensors.value];
}

function changed() {
  config.markDirty();
}

function edit(index: 0 | 1, update: (slot: ReadoutSlot) => void) {
  update(ensureSlots(props.overlay)[index]);
  changed();
}

function setKind(index: 0 | 1, kind: ReadoutKind) {
  edit(index, (slot) => {
    slot.kind = kind;
    delete slot.source;
    if (kind === "clock") slot.sparkline = false;
  });
}

function setSource(index: 0 | 1, value: string) {
  edit(index, (slot) => {
    const source = value === AUTO ? null : decodeOption(value);
    if (source) slot.source = source;
    else delete slot.source;
  });
}

function setLabel(index: 0 | 1, value: string) {
  edit(index, (slot) => {
    const label = value.slice(0, READOUT_LABEL_MAX);
    if (label.trim()) slot.label = label;
    else delete slot.label;
  });
}

function setNumber(key: "warm_c" | "hot_c" | "reserve_px", value: number | null, fallback: number) {
  props.overlay[key] = value ?? fallback;
  changed();
}

function setFont(path: string) {
  if (path) props.overlay.font_path = path;
  else delete props.overlay.font_path;
  changed();
}

function hasSensor(kind: ReadoutKind): boolean {
  return kind !== "off" && kind !== "clock";
}

const thresholdError = computed(() => props.overlay.hot_c < props.overlay.warm_c);
const enabled = computed(() => overlayEnabled(props.overlay));
</script>

<template>
  <div class="readouts">
    <div v-for="(slot, index) in slots" :key="index" class="slot">
      <label class="muted">{{ t(POSITION_LABELS[index]) }}</label>
      <div class="slot-grid">
        <n-select :value="slot.kind" :options="kindOptions" size="small" @update:value="(kind: ReadoutKind) => setKind(index as 0 | 1, kind)" />
        <SensorSelect
          v-if="hasSensor(slot.kind)"
          :value="slot.source ? JSON.stringify(slot.source) : slot.kind === 'sensor' ? null : AUTO"
          :options="sensorOptions(slot.kind)"
          :placeholder="t('lcd.readouts.chooseSensor')"
          size="small"
          @update:value="(value: string) => setSource(index as 0 | 1, value)"
        />
        <n-input
          v-if="slot.kind !== 'off'"
          :value="slot.label ?? ''"
          size="small"
          :maxlength="READOUT_LABEL_MAX"
          :placeholder="previews?.[index]?.label || t('lcd.readouts.label')"
          @update:value="(value: string) => setLabel(index as 0 | 1, value)"
        />
        <n-checkbox
          v-if="hasSensor(slot.kind)"
          :checked="slot.sparkline"
          @update:checked="(value: boolean) => edit(index as 0 | 1, (s) => { s.sparkline = value; })"
        >{{ t("lcd.readouts.sparkline") }}</n-checkbox>
      </div>
      <p v-if="slot.kind !== 'off' && previews?.[index]" class="hint">
        {{ previews[index].sensor
          ? t("lcd.readouts.nowFrom", { text: previews[index].text, sensor: previews[index].sensor })
          : t("lcd.readouts.now", { text: previews[index].text }) }}
      </p>
      <p v-if="slot.kind === 'sensor' && !slot.source" class="error">{{ t("lcd.readouts.sensorRequired") }}</p>
    </div>

    <template v-if="enabled">
      <div class="style-grid">
        <n-checkbox :checked="overlay.fahrenheit" @update:checked="(v: boolean) => { overlay.fahrenheit = v; changed(); }">
          {{ t("lcd.readouts.fahrenheit") }}
        </n-checkbox>
        <n-checkbox :checked="overlay.colors" @update:checked="(v: boolean) => { overlay.colors = v; changed(); }">
          {{ t("lcd.readouts.colors") }}
        </n-checkbox>
      </div>
      <div v-if="overlay.colors" class="style-grid">
        <div class="field">
          <label class="muted">{{ t("lcd.readouts.warmAt") }}</label>
          <n-input-number :value="overlay.warm_c" size="small" :min="0" :max="150" @update:value="(v: number | null) => setNumber('warm_c', v, 75)" />
        </div>
        <div class="field">
          <label class="muted">{{ t("lcd.readouts.hotAt") }}</label>
          <n-input-number :value="overlay.hot_c" size="small" :min="0" :max="150" @update:value="(v: number | null) => setNumber('hot_c', v, 85)" />
        </div>
      </div>
      <p v-if="overlay.colors && thresholdError" class="error">{{ t("lcd.readouts.thresholdOrder") }}</p>
      <div class="style-grid">
        <div class="field">
          <label class="muted">{{ t("lcd.readouts.font") }}</label>
          <n-select :value="overlay.font_path ?? ''" :options="fontOptions()" size="small" filterable @update:value="setFont" />
        </div>
        <div class="field">
          <n-checkbox
            :checked="overlay.reserve_px > 0"
            @update:checked="(v: boolean) => setNumber('reserve_px', v ? RESERVE_PX.default : 0, 0)"
          >{{ t("lcd.readouts.reserve") }}</n-checkbox>
          <n-input-number
            v-if="overlay.reserve_px > 0"
            :value="overlay.reserve_px"
            size="small"
            :min="RESERVE_PX.min"
            :max="RESERVE_PX.max"
            @update:value="(v: number | null) => setNumber('reserve_px', v, RESERVE_PX.default)"
          >
            <template #suffix>px</template>
          </n-input-number>
        </div>
      </div>
      <p class="hint">{{ t("lcd.readouts.hint") }}</p>
      <n-alert v-if="fit !== 'contain'" type="warning" :show-icon="false">{{ t("lcd.readouts.needsFit") }}</n-alert>
    </template>
  </div>
</template>

<style scoped>
.readouts {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.slot {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.slot-grid,
.style-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  align-items: center;
  gap: var(--space-2);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
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

<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { backendText, invoke, type UiMessage } from "@/utils/backendText";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import { lcdEntryKey, resolveLcdDevice } from "@/utils/lcdSelection";
import type { LcdConfig, LcdTemplate } from "@/types";
import { useTranslate, type MessageKey } from "@/i18n";

const props = defineProps<{ lcds: LcdConfig[]; templates: LcdTemplate[] }>();
const emit = defineEmits<{ inProgress: [active: boolean] }>();
interface Result { import_id: string; lcds: LcdConfig[]; templates: LcdTemplate[] }
interface Status { id: string; active: boolean; result: Result | null; error: string | null; error_ui?: UiMessage | null }
const daemon = useDaemonStore();
const config = useConfigStore();
const devices = useDevicesStore();
const names = useDeviceNamesStore();
const t = useTranslate();

function screenName(entry: LcdConfig): string {
  const device = resolveLcdDevice(entry, devices.lcdDevices);
  const label = device ? devices.labels.get(device.device_id) ?? device.name : null;
  return names.lcdEntryName(entry) || label || t("lcd.screenOffline");
}

const incomplete = computed(() =>
  props.lcds.flatMap((entry) => {
    if (["image", "gif", "video"].includes(entry.type) && !entry.path) return [t("lcd.managedImport.chooseFile", { screen: screenName(entry) })];
    if (entry.type === "custom" && !entry.template_id) return [t("lcd.managedImport.chooseTemplate", { screen: screenName(entry) })];
    if (entry.type === "playlist" && !entry.playlist?.items.length) return [t("lcd.managedImport.chooseFile", { screen: screenName(entry) })];
    return [];
  }),
);

function readableError(message: string): string {
  return message.replace(/LCD\[(serial|index):([^\]]+)\]/g, (match, kind: string, id: string) => {
    const key = kind === "serial" ? `serial:${id.replace(/^hid:/, "")}` : `index:${id}`;
    const entry = props.lcds.find((candidate) => lcdEntryKey(candidate) === key);
    return entry ? screenName(entry) : match;
  });
}

const status = ref<Status | null>(null);
const busy = ref(false);
const error = ref("");
const notice = ref<MessageKey | "">("");
const confirmed = ref(false);
const pending = ref("");
const stagedId = ref("");
let revision = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let pendingChecks = 0;

function reset() {
  revision++;
  clearTimeout(timer);
  status.value = null;
  pending.value = "";
  stagedId.value = "";
  pendingChecks = 0;
  busy.value = false;
  error.value = "";
  notice.value = "";
  confirmed.value = false;
}
watch([() => daemon.connected, () => daemon.info?.instance_id], reset);
watch(
  () =>
    busy.value ||
    !!pending.value ||
    !!status.value?.active ||
    (!!status.value?.result && status.value.id !== stagedId.value),
  (active) => emit("inProgress", active),
  { immediate: true },
);
onUnmounted(reset);

async function check() {
  if (busy.value) return;
  const current = revision;
  busy.value = true;
  clearTimeout(timer);
  try {
    const value = await invoke<Status | null>("managed_import_status");
    if (current !== revision) return;
    if (value && (!/^[a-f0-9]{32}$/i.test(value.id) || typeof value.active !== "boolean")) throw new Error(t("lcd.managedImport.invalidProgress"));
    if (pending.value && value?.id !== pending.value) {
      notice.value = "lcd.managedImport.notReported";
      if (++pendingChecks < 30) timer = setTimeout(() => void check(), 1000);
      return;
    }
    status.value = value;
    pending.value = "";
    pendingChecks = 0;
    error.value = "";
    notice.value = value?.active ? "lcd.managedImport.copying"
      : value?.result ? "lcd.managedImport.ready"
        : value ? "lcd.managedImport.incomplete" : "lcd.managedImport.noImport";
    if (value?.active) timer = setTimeout(() => void check(), 1000);
  } catch (reason) {
    if (current === revision) error.value = String(reason);
  } finally {
    if (current === revision) busy.value = false;
  }
}

async function start() {
  if (busy.value || status.value?.active || pending.value || !daemon.info || config.imported || incomplete.value.length) return;
  const current = ++revision;
  clearTimeout(timer);
  busy.value = true;
  error.value = "";
  status.value = null;
  confirmed.value = false;
  notice.value = "lcd.managedImport.submitting";
  try {
    const id = await invoke<string>("managed_import_start", { instance: daemon.info.instance_id, lcds: props.lcds, templates: props.templates });
    if (current !== revision) return;
    pending.value = id;
    notice.value = "lcd.managedImport.submitted";
    timer = setTimeout(() => void check(), 1000);
  } catch (reason) {
    if (current === revision) { error.value = String(reason); notice.value = "lcd.managedImport.notConfirmed"; }
  } finally {
    if (current === revision) busy.value = false;
  }
}

async function stage() {
  if (busy.value || !confirmed.value || !status.value?.result || !daemon.info) return;
  const current = revision;
  const instance = daemon.info.instance_id;
  busy.value = true;
  try {
    const result = await invoke<Result>("managed_import_result", { instance, id: status.value.id });
    if (current !== revision) return;
    config.stageImportedMedia(result.lcds, result.templates, instance);
    stagedId.value = status.value?.id ?? "";
    confirmed.value = false;
    notice.value = "lcd.managedImport.staged";
  } catch (reason) {
    if (current === revision) error.value = String(reason);
  } finally {
    if (current === revision) busy.value = false;
  }
}
</script>

<template>
  <n-card :title="t('lcd.managedImport.title')" size="small">
    <p>{{ t("lcd.managedImport.description") }}</p>
    <n-space>
      <n-button :disabled="busy || !daemon.connected || !lcds.length || !!incomplete.length || !!config.imported || !!status?.active || !!pending" @click="start">{{ t("lcd.managedImport.copy") }}</n-button>
      <n-button :disabled="busy" @click="check">{{ t("lcd.managedImport.check") }}</n-button>
    </n-space>
    <p v-if="notice" role="status">{{ t(notice) }}</p>
    <n-alert v-if="incomplete.length" type="warning" :title="t('lcd.managedImport.incompleteTitle')">
      <ul class="incomplete"><li v-for="item in incomplete" :key="item">{{ item }}</li></ul>
    </n-alert>
    <n-alert v-if="error" type="error">{{ readableError(error) }}</n-alert>
    <n-alert v-else-if="status?.error" type="error">{{ t("lcd.managedImport.importFailed", { error: readableError(backendText(status.error, status.error_ui)) }) }}</n-alert>
    <template v-if="status?.result">
      <p>{{ t("lcd.managedImport.resultSummary", { lcds: status.result.lcds.length, templates: status.result.templates.length }) }}</p>
      <ul><li v-for="(lcd, index) in status.result.lcds" :key="index">{{ t("lcd.managedImport.resultItem", { screen: screenName(lcd), value: lcd.path ?? lcd.template_id ?? lcd.type }) }}</li></ul>
      <n-checkbox v-model:checked="confirmed" :disabled="busy || !!config.imported">{{ t("lcd.managedImport.confirm") }}</n-checkbox>
      <n-button :disabled="busy || !confirmed || !!config.imported || !daemon.connected" @click="stage">{{ t("lcd.managedImport.stage") }}</n-button>
    </template>
  </n-card>
</template>

<style scoped>
p { margin: var(--space-2) 0; }
li { overflow-wrap: anywhere; }
.incomplete { margin: 0; padding-left: var(--space-4); }
</style>

<script setup lang="ts">
import { nextTick, ref, watch } from "vue";

interface SheetTabItem {
  name: string;
}

const props = withDefaults(
  defineProps<{
    sheets: SheetTabItem[];
    active: string;
    adding?: boolean;
  }>(),
  { adding: false },
);

const emit = defineEmits<{
  (event: "select", name: string): void;
  (event: "add"): void;
  (event: "rename", oldName: string, newName: string): void;
}>();

const rootRef = ref<HTMLElement | null>(null);
const renaming = ref("");
const draft = ref("");
const editWidth = ref(120);
let measureCanvas: HTMLCanvasElement | null = null;

function measureTextWidth(text: string, font: string): number {
  if (!measureCanvas) measureCanvas = document.createElement("canvas");
  const context = measureCanvas.getContext("2d");
  if (!context) return text.length * 7.2;
  context.font = font;
  return context.measureText(text).width;
}

function measureTabWidth(text: string): number {
  const tab = rootRef.value?.querySelector<HTMLElement>(".sheet-tab");
  const style = tab ? getComputedStyle(tab) : null;
  const font = style
    ? `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`
    : '12px "PingFang SC", "Microsoft YaHei", sans-serif';
  const width = measureTextWidth(text || " ", font);
  return Math.min(240, Math.max(32, Math.ceil(width) + 10));
}

function updateEditWidth(text?: string) {
  editWidth.value = measureTabWidth(text ?? draft.value);
}

function startRename(name: string) {
  renaming.value = name;
  draft.value = name;
  updateEditWidth(name);
  void nextTick(() => {
    const input = rootRef.value?.querySelector<HTMLInputElement>(".sheet-tab-input");
    input?.focus();
    input?.select();
  });
}

function cancelRename() {
  renaming.value = "";
  draft.value = "";
}

function commitRename() {
  const oldName = renaming.value;
  if (!oldName) return;
  const newName = draft.value.trim();
  renaming.value = "";
  draft.value = "";
  if (!newName || newName === oldName) return;
  emit("rename", oldName, newName);
}

/** 中文输入法组词阶段 v-model 还没更新，直接读输入框当前文本撑开宽度。 */
function onInput(event: Event) {
  const input = event.target as HTMLInputElement;
  const width = measureTabWidth(input.value);
  input.style.width = `${width}px`;
  editWidth.value = width;
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    event.stopPropagation();
    commitRename();
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    cancelRename();
  }
}

function tabStyle(name: string): Record<string, string> {
  return renaming.value === name
    ? { "--sheet-tab-width": `${editWidth.value}px` }
    : {};
}

watch(draft, () => {
  if (renaming.value) updateEditWidth();
});

watch(
  () => props.sheets,
  (sheets) => {
    if (renaming.value && !sheets.some((sheet) => sheet.name === renaming.value)) {
      cancelRename();
    }
  },
);
</script>

<template>
  <div ref="rootRef" class="sheet-tabs">
    <div
      v-for="sheet in sheets"
      :key="sheet.name"
      class="sheet-tab"
      :class="{ active: sheet.name === active, editing: renaming === sheet.name }"
      :style="tabStyle(sheet.name)"
      role="button"
      tabindex="0"
      @click="emit('select', sheet.name)"
      @keydown.enter="emit('select', sheet.name)"
      @dblclick.stop="startRename(sheet.name)"
    >
      <input
        v-if="renaming === sheet.name"
        v-model="draft"
        class="sheet-tab-input"
        @input="onInput"
        @compositionupdate="onInput"
        @compositionend="onInput"
        @click.stop
        @dblclick.stop
        @keydown="onKeydown"
        @blur="commitRename"
      />
      <span v-else class="sheet-tab-name">{{ sheet.name }}</span>
    </div>

    <button
      class="sheet-add sheet-add-icon"
      type="button"
      title="新建工作表"
      aria-label="新建工作表"
      :disabled="adding"
      @click="emit('add')"
    >
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M8 3v10M3 8h10" />
      </svg>
    </button>
  </div>
</template>

<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  useSlots,
  watch,
} from "vue";
import Sortable from "sortablejs";
import type { SheetRowData } from "../types";

type MoveDirection = "up" | "down" | "left" | "right";
type CellPosition = { row: number; col: number };

const DRAG_WIDTH = 26;
const ACTIONS_WIDTH = 72;

const props = withDefaults(
  defineProps<{
    headers: string[];
    rows: SheetRowData[];
    /** 计算列宽时参考的完整数据；不传则用 rows。 */
    sizingRows?: SheetRowData[];
    loading?: boolean;
    emptyText?: string;
    editable?: boolean;
    draggable?: boolean;
    resizable?: boolean;
    dragDisabled?: boolean;
    minWidth?: number;
    /** 列宽缓存键；为空时不持久化。 */
    widthCacheKey?: string;
    align?: ("left" | "right")[];
  }>(),
  {
    sizingRows: undefined,
    loading: false,
    emptyText: "没有数据",
    editable: true,
    draggable: true,
    resizable: true,
    dragDisabled: false,
    minWidth: 640,
    widthCacheKey: "",
    align: () => [],
  },
);

const emit = defineEmits<{
  (event: "cell-change", payload: { row: number; col: number; value: string }): void;
  (event: "rows-reorder", payload: { oldIndex: number; newIndex: number }): void;
}>();

const slots = useSlots();
const hasActions = computed(() => Boolean(slots["row-actions"]));

const gridRef = ref<HTMLElement | null>(null);
const tbodyRef = ref<HTMLElement | null>(null);
const selection = ref<CellPosition | null>(null);
const editingCell = ref<CellPosition | null>(null);
const cellDraft = ref("");
let seededEdit = false;

const manualWidths = ref<Record<number, number>>({});
let resizeState: { index: number; startX: number; startWidth: number } | null = null;
let rowSortable: Sortable | null = null;

function displayUnits(text: string): number {
  let units = 0;
  for (const char of text) {
    units += char.charCodeAt(0) > 0xff ? 2 : 1;
  }
  return units;
}

// 按表头和整表内容估算列宽，超长内容会被截断而不是无限撑宽。
const autoWidths = computed(() => {
  const rows = props.sizingRows ?? props.rows;
  return props.headers.map((header, col) => {
    let units = displayUnits(header);
    for (const row of rows) {
      const cellUnits = Math.min(52, displayUnits(row.cells[col] ?? ""));
      if (cellUnits > units) units = cellUnits;
      if (units >= 52) break;
    }
    return Math.min(380, Math.max(84, Math.round(units * 6.8 + 30)));
  });
});

const columnWidths = computed(() =>
  props.headers.map(
    (_, index) => manualWidths.value[index] ?? autoWidths.value[index],
  ),
);

const tableWidth = computed(() => {
  const columns = columnWidths.value.reduce((sum, width) => sum + width, 0);
  return (
    columns +
    (props.draggable ? DRAG_WIDTH : 0) +
    (hasActions.value ? ACTIONS_WIDTH : 0)
  );
});

const gridTemplate = computed(() =>
  [
    ...(props.draggable ? [`${DRAG_WIDTH}px`] : []),
    ...columnWidths.value.map((width) => `${width}px`),
    "1fr",
    ...(hasActions.value ? [`${ACTIONS_WIDTH}px`] : []),
  ].join(" "),
);

const contentMinWidth = computed(() => Math.max(props.minWidth, tableWidth.value));

function loadWidths() {
  manualWidths.value = {};
  if (!props.widthCacheKey) return;
  try {
    const raw = localStorage.getItem(props.widthCacheKey);
    if (!raw) return;
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const widths: Record<number, number> = {};
    for (const [key, value] of Object.entries(parsed)) {
      const index = Number(key);
      const width = Number(value);
      if (Number.isInteger(index) && Number.isFinite(width)) {
        widths[index] = Math.min(600, Math.max(60, Math.round(width)));
      }
    }
    manualWidths.value = widths;
  } catch {
    manualWidths.value = {};
  }
}

function saveWidths() {
  if (!props.widthCacheKey) return;
  try {
    localStorage.setItem(
      props.widthCacheKey,
      JSON.stringify(manualWidths.value),
    );
  } catch {
    // 本地缓存不可用时忽略，宽度只在本次运行内有效。
  }
}

function resetWidths() {
  manualWidths.value = {};
  saveWidths();
}

function startResize(event: MouseEvent, index: number) {
  event.preventDefault();
  event.stopPropagation();
  if (event.detail >= 2) {
    const next = { ...manualWidths.value };
    delete next[index];
    manualWidths.value = next;
    saveWidths();
    return;
  }
  const header = (event.currentTarget as HTMLElement).parentElement;
  const startWidth =
    header?.getBoundingClientRect().width ?? columnWidths.value[index] ?? 100;
  resizeState = { index, startX: event.clientX, startWidth };
  document.body.classList.add("col-resizing");
  window.addEventListener("mousemove", onResizeMove);
  window.addEventListener("mouseup", stopResize);
}

function onResizeMove(event: MouseEvent) {
  if (!resizeState) return;
  const width = Math.min(
    600,
    Math.max(60, resizeState.startWidth + event.clientX - resizeState.startX),
  );
  manualWidths.value = {
    ...manualWidths.value,
    [resizeState.index]: Math.round(width),
  };
}

function stopResize() {
  const wasResizing = resizeState !== null;
  resizeState = null;
  document.body.classList.remove("col-resizing");
  window.removeEventListener("mousemove", onResizeMove);
  window.removeEventListener("mouseup", stopResize);
  if (wasResizing) saveWidths();
}

function cellAt(row: number, col: number): string {
  return props.rows.find((item) => item.row === row)?.cells[col] ?? "";
}

function isSelected(row: number, col: number): boolean {
  return selection.value?.row === row && selection.value.col === col;
}

function isEditing(row: number, col: number): boolean {
  return editingCell.value?.row === row && editingCell.value.col === col;
}

function focusGrid() {
  void nextTick(() => gridRef.value?.focus({ preventScroll: true }));
}

function selectCell(row: number, col: number) {
  selection.value = { row, col };
  void nextTick(() => {
    const element = gridRef.value?.querySelector<HTMLElement>(
      `[data-cell="${row}-${col}"]`,
    );
    element?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });
}

function onCellClick(row: number, col: number) {
  if (editingCell.value?.row === row && editingCell.value.col === col) return;
  selectCell(row, col);
  focusGrid();
}

function moveSelection(rowDelta: number, colDelta: number) {
  const current = selection.value;
  if (!current) return;
  const rows = props.rows;
  const index = rows.findIndex((item) => item.row === current.row);
  const maxCol = props.headers.length - 1;
  if (index < 0 || maxCol < 0) return;
  const nextRowIndex = Math.min(rows.length - 1, Math.max(0, index + rowDelta));
  const nextCol = Math.min(maxCol, Math.max(0, current.col + colDelta));
  selectCell(rows[nextRowIndex].row, nextCol);
  focusGrid();
}

function startEdit(row: number, col: number, seed?: string) {
  if (!props.editable) return;
  if (editingCell.value?.row === row && editingCell.value.col === col) return;
  selection.value = { row, col };
  seededEdit = seed !== undefined;
  cellDraft.value = seed ?? cellAt(row, col);
  editingCell.value = { row, col };
}

function cancelEdit() {
  editingCell.value = null;
  focusGrid();
}

function commitCell(direction?: MoveDirection, shouldFocus = true) {
  const cell = editingCell.value;
  if (!cell) return;
  const value = cellDraft.value.trim();
  editingCell.value = null;
  if (value !== cellAt(cell.row, cell.col)) {
    emit("cell-change", { row: cell.row, col: cell.col, value });
  }

  if (direction) {
    selection.value = { ...cell };
    const deltas: Record<MoveDirection, [number, number]> = {
      up: [-1, 0],
      down: [1, 0],
      left: [0, -1],
      right: [0, 1],
    };
    const [rowDelta, colDelta] = deltas[direction];
    moveSelection(rowDelta, colDelta);
  } else if (shouldFocus) {
    selection.value = { ...cell };
    focusGrid();
  }
}

function clearCell(row: number, col: number) {
  if (!props.editable) return;
  if (cellAt(row, col) === "") return;
  emit("cell-change", { row, col, value: "" });
}

function onEditorKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    commitCell(event.shiftKey ? "up" : "down");
    return;
  }
  if (event.key === "Tab") {
    event.preventDefault();
    commitCell(event.shiftKey ? "left" : "right");
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    cancelEdit();
  }
}

function onGridKeydown(event: KeyboardEvent) {
  if (editingCell.value || !selection.value) return;
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  const origin = event.target as HTMLElement | null;
  if (origin?.closest("button, input, textarea, select")) return;
  const current = selection.value;
  switch (event.key) {
    case "ArrowUp":
      event.preventDefault();
      moveSelection(-1, 0);
      break;
    case "ArrowDown":
      event.preventDefault();
      moveSelection(1, 0);
      break;
    case "ArrowLeft":
      event.preventDefault();
      moveSelection(0, -1);
      break;
    case "ArrowRight":
      event.preventDefault();
      moveSelection(0, 1);
      break;
    case "Tab":
      event.preventDefault();
      moveSelection(0, event.shiftKey ? -1 : 1);
      break;
    case "Enter":
      event.preventDefault();
      moveSelection(event.shiftKey ? -1 : 1, 0);
      break;
    case "F2":
      event.preventDefault();
      startEdit(current.row, current.col);
      break;
    case "Delete":
    case "Backspace":
      if (!props.editable) return;
      event.preventDefault();
      clearCell(current.row, current.col);
      break;
    default:
      if (props.editable && event.key.length === 1) {
        event.preventDefault();
        startEdit(current.row, current.col, event.key);
      }
  }
}

watch(editingCell, async (cell) => {
  if (!cell) return;
  await nextTick();
  const input = gridRef.value?.querySelector<HTMLInputElement>(".cell-editor");
  if (!input) return;
  input.focus({ preventScroll: true });
  if (seededEdit) {
    const end = input.value.length;
    input.setSelectionRange(end, end);
  } else {
    input.select();
  }
});

function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (!target || target.closest(".ledger-grid-cell")) return;
  selection.value = null;
}

function initRowSortable() {
  rowSortable?.destroy();
  rowSortable = null;
  const el = tbodyRef.value;
  if (!el || !props.draggable) return;
  rowSortable = Sortable.create(el, {
    handle: ".drag-handle",
    animation: 200,
    easing: "cubic-bezier(0.22, 1, 0.36, 1)",
    ghostClass: "drag-ghost",
    chosenClass: "drag-chosen",
    dragClass: "drag-active",
    fallbackClass: "drag-fallback",
    forceFallback: true,
    fallbackOnBody: true,
    fallbackTolerance: 3,
    onStart() {
      el.classList.add("is-sorting");
      selection.value = null;
      editingCell.value = null;
    },
    onEnd(event) {
      el.classList.remove("is-sorting");
      window.getSelection()?.removeAllRanges();
      const { oldIndex, newIndex } = event;
      if (oldIndex == null || newIndex == null || oldIndex === newIndex) return;
      emit("rows-reorder", { oldIndex, newIndex });
    },
  });
  rowSortable.option("disabled", props.dragDisabled);
}

watch(tbodyRef, initRowSortable, { flush: "post" });
watch(() => props.draggable, initRowSortable);
watch(
  () => props.dragDisabled,
  (disabled) => rowSortable?.option("disabled", disabled),
);
watch(() => props.widthCacheKey, loadWidths);

onMounted(() => {
  loadWidths();
  document.addEventListener("click", onDocumentClick);
});

onBeforeUnmount(() => {
  rowSortable?.destroy();
  rowSortable = null;
  document.removeEventListener("click", onDocumentClick);
  window.removeEventListener("mousemove", onResizeMove);
  window.removeEventListener("mouseup", stopResize);
  document.body.classList.remove("col-resizing");
});

defineExpose({
  clearInteraction() {
    selection.value = null;
    editingCell.value = null;
  },
  resetWidths,
});
</script>

<template>
  <div
    ref="gridRef"
    class="table-wrap ledger-table-wrap"
    tabindex="0"
    @keydown="onGridKeydown"
  >
    <div
      class="ledger-grid"
      :style="{ minWidth: `${contentMinWidth}px` }"
    >
      <div class="ledger-grid-head" :style="{ gridTemplateColumns: gridTemplate }">
        <div v-if="draggable" class="drag-col"></div>
        <div
          v-for="(header, index) in headers"
          :key="index"
          class="col-head"
          :class="{ 'cell-right': align[index] === 'right' }"
          :title="header"
        >
          <span class="th-label">{{ header }}</span>
          <span
            v-if="resizable"
            class="col-resizer"
            title="拖动调整列宽，双击恢复自适应"
            @mousedown="startResize($event, index)"
          ></span>
        </div>
        <div class="filler-col"></div>
        <div v-if="hasActions" class="actions-col">操作</div>
      </div>

      <div ref="tbodyRef" class="ledger-grid-body">
        <div
          v-for="row in rows"
          :key="row.row"
          class="ledger-grid-row"
          :style="{ gridTemplateColumns: gridTemplate }"
        >
          <div v-if="draggable" class="drag-col">
            <span
              class="drag-handle"
              :class="{ disabled: dragDisabled }"
              :title="dragDisabled ? '当前状态下不能调整顺序' : '拖动调整顺序'"
            >
              ⠿
            </span>
          </div>
          <div
            v-for="(_, index) in headers"
            :key="index"
            class="ledger-grid-cell"
            :class="{
              'cell-selected': isSelected(row.row, index),
              'cell-editing': isEditing(row.row, index),
              'cell-right': align[index] === 'right',
            }"
            :data-cell="`${row.row}-${index}`"
            :title="row.cells[index] ?? ''"
            @click="onCellClick(row.row, index)"
            @dblclick="startEdit(row.row, index)"
          >
            <input
              v-if="isEditing(row.row, index)"
              v-model="cellDraft"
              class="cell-editor"
              @keydown="onEditorKeydown"
              @blur="commitCell(undefined, false)"
            />
            <span v-else class="cell-text">{{ row.cells[index] ?? "" }}</span>
          </div>
          <div class="filler-col"></div>
          <div v-if="hasActions" class="actions-col">
            <div class="row-actions">
              <slot name="row-actions" :row="row" />
            </div>
          </div>
        </div>
      </div>
    </div>
    <div v-if="loading" class="table-empty">加载中</div>
    <div v-else-if="rows.length === 0" class="table-empty">{{ emptyText }}</div>
  </div>
</template>

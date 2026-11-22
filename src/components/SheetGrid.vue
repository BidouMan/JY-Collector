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
type DisplayItem =
  | {
      kind: "group";
      name: string;
      count: number;
      depth: number;
      path: string;
    }
  | { kind: "row"; row: SheetRowData };

const DRAG_WIDTH = 26;
const ACTIONS_WIDTH = 72;
const SELECT_WIDTH = 32;

const props = withDefaults(
  defineProps<{
    headers: string[];
    rows: SheetRowData[];
    /** 计算列宽时参考的完整数据；不传则用 rows。 */
    sizingRows?: SheetRowData[];
    /** 与 headers 对应的模板列 key。 */
    keys?: string[];
    /** 与 headers 对应的列类型：text / number / date / select / multi_select。 */
    formats?: string[];
    /** 与 headers 对应的单选/多选候选项。 */
    options?: (string[] | undefined)[];
    /** 需要隐藏的原始列下标。 */
    hiddenColumns?: number[];
    loading?: boolean;
    emptyText?: string;
    editable?: boolean;
    /** 是否允许双击表头改列名、列菜单与新增列按钮。 */
    columnEditable?: boolean;
    draggable?: boolean;
    resizable?: boolean;
    dragDisabled?: boolean;
    minWidth?: number;
    /** 列宽缓存键；为空时不持久化。 */
    widthCacheKey?: string;
    align?: ("left" | "right")[];
    /** 冻结第一个可见数据列。 */
    freezeFirstColumn?: boolean;
    /** 按这些列逐级分组（原始列下标）；空数组表示不分组。 */
    groupBy?: number[];
    /** 与 groupBy 对应的分组方向。 */
    groupDirections?: ("asc" | "desc")[];
    /** 是否显示行多选复选框。 */
    selectableRows?: boolean;
    /** 行操作列宽度。 */
    actionsWidth?: number;
  }>(),
  {
    sizingRows: undefined,
    keys: () => [],
    formats: () => [],
    options: () => [],
    hiddenColumns: () => [],
    loading: false,
    emptyText: "没有数据",
    editable: true,
    columnEditable: false,
    draggable: true,
    resizable: true,
    dragDisabled: false,
    minWidth: 640,
    widthCacheKey: "",
    align: () => [],
    freezeFirstColumn: false,
    groupBy: () => [],
    groupDirections: () => [],
    selectableRows: false,
    actionsWidth: ACTIONS_WIDTH,
  },
);

const emit = defineEmits<{
  (event: "cell-change", payload: { row: number; col: number; value: string }): void;
  (event: "rows-reorder", payload: { oldIndex: number; newIndex: number }): void;
  (event: "column-rename", payload: { index: number; value: string }): void;
  (event: "column-add"): void;
  (event: "column-action", payload: { action: string; index: number }): void;
  (event: "range-paste", payload: { row: number; col: number; values: string[][] }): void;
  (event: "range-clear", payload: { cells: { row: number; col: number }[] }): void;
  (
    event: "range-fill",
    payload: {
      rowStart: number;
      rowEnd: number;
      colStart: number;
      colEnd: number;
      direction: "down" | "right";
    },
  ): void;
  (event: "cell-focus", payload: { row: number; col: number }): void;
  (
    event: "cell-contextmenu",
    payload: { row: number; col: number; x: number; y: number },
  ): void;
  (event: "selection-change", payload: number[]): void;
}>();

const slots = useSlots();
const hasActions = computed(() => Boolean(slots["row-actions"]));
const hiddenSet = computed(() => new Set(props.hiddenColumns));
const visibleCols = computed(() =>
  props.headers.map((_, index) => index).filter((index) => !hiddenSet.value.has(index)),
);

const gridRef = ref<HTMLElement | null>(null);
const tbodyRef = ref<HTMLElement | null>(null);
const anchor = ref<CellPosition | null>(null);
const focusCell = ref<CellPosition | null>(null);
const editingCell = ref<CellPosition | null>(null);
const cellDraft = ref("");
const renamingCol = ref<number | null>(null);
const columnDraft = ref("");
const selectedRows = ref<Set<number>>(new Set());
const collapsedGroups = ref<Set<string>>(new Set());
const menuState = ref<{ index: number; x: number; y: number } | null>(null);
let seededEdit = false;
let draggingSelection = false;

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
    return Math.min(380, Math.max(96, Math.round(units * 7 + 38)));
  });
});

const columnWidths = computed(() =>
  props.headers.map(
    (_, index) => manualWidths.value[index] ?? autoWidths.value[index],
  ),
);

const tableWidth = computed(() => {
  const columns = visibleCols.value.reduce(
    (sum, index) => sum + (columnWidths.value[index] ?? 84),
    0,
  );
  return (
    columns +
    (props.selectableRows ? SELECT_WIDTH : 0) +
    (props.draggable ? DRAG_WIDTH : 0) +
    (hasActions.value ? props.actionsWidth : 0)
  );
});

const gridTemplate = computed(() =>
  [
    ...(props.selectableRows ? [`${SELECT_WIDTH}px`] : []),
    ...(props.draggable ? [`${DRAG_WIDTH}px`] : []),
    ...visibleCols.value.map((index) => `${columnWidths.value[index] ?? 84}px`),
    "1fr",
    ...(hasActions.value ? [`${props.actionsWidth}px`] : []),
  ].join(" "),
);

const contentMinWidth = computed(() => Math.max(props.minWidth, tableWidth.value));

const frozenCol = computed(() =>
  props.freezeFirstColumn ? visibleCols.value[0] : undefined,
);
const frozenLeft = computed(
  () => (props.selectableRows ? SELECT_WIDTH : 0) + (props.draggable ? DRAG_WIDTH : 0),
);

function formatOf(col: number): string {
  return props.formats[col] ?? "text";
}

function optionsOf(col: number): string[] {
  return props.options[col] ?? [];
}

function rightAligned(col: number): boolean {
  return props.align[col] === "right" || formatOf(col) === "number";
}

function splitTags(value: string): string[] {
  return value
    .split(/[,，]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function compareGroupNames(left: string, right: string): number {
  const leftNumber = Number(left);
  const rightNumber = Number(right);
  if (
    left !== "" &&
    right !== "" &&
    Number.isFinite(leftNumber) &&
    Number.isFinite(rightNumber)
  ) {
    return leftNumber - rightNumber;
  }
  return left.localeCompare(right, "zh-CN");
}

function buildGroupItems(
  rows: SheetRowData[],
  level: number,
  parentPath: string,
  items: DisplayItem[],
) {
  if (level >= props.groupBy.length) {
    for (const row of rows) items.push({ kind: "row", row });
    return;
  }
  const col = props.groupBy[level];
  const map = new Map<string, SheetRowData[]>();
  for (const row of rows) {
    const label = (row.cells[col] ?? "").trim() || "（空）";
    const bucket = map.get(label);
    if (bucket) bucket.push(row);
    else map.set(label, [row]);
  }
  const entries = [...map.entries()];
  const direction = props.groupDirections[level] ?? "asc";
  if (direction === "desc") {
    entries.sort((a, b) => compareGroupNames(b[0], a[0]));
  } else {
    entries.sort((a, b) => compareGroupNames(a[0], b[0]));
  }
  for (const [name, groupRows] of entries) {
    const path = `${parentPath}\u0001${level}:${name}`;
    items.push({
      kind: "group",
      name,
      count: groupRows.length,
      depth: level,
      path,
    });
    if (collapsedGroups.value.has(path)) continue;
    buildGroupItems(groupRows, level + 1, path, items);
  }
}

const displayItems = computed<DisplayItem[]>(() => {
  if (props.groupBy.length === 0) {
    return props.rows.map((row) => ({ kind: "row", row }));
  }
  const items: DisplayItem[] = [];
  buildGroupItems(props.rows, 0, "", items);
  return items;
});

function toggleGroup(path: string) {
  const next = new Set(collapsedGroups.value);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  collapsedGroups.value = next;
}

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

function rowIndex(row: number): number {
  return props.rows.findIndex((item) => item.row === row);
}

function rangeInfo() {
  const a = anchor.value;
  const f = focusCell.value;
  if (!a || !f) return null;
  const ai = rowIndex(a.row);
  const fi = rowIndex(f.row);
  if (ai < 0 || fi < 0) return null;
  return {
    rowStart: Math.min(ai, fi),
    rowEnd: Math.max(ai, fi),
    colStart: Math.min(a.col, f.col),
    colEnd: Math.max(a.col, f.col),
  };
}

function inRange(row: number, col: number): boolean {
  const info = rangeInfo();
  if (!info) return false;
  const index = rowIndex(row);
  return (
    index >= info.rowStart &&
    index <= info.rowEnd &&
    col >= info.colStart &&
    col <= info.colEnd
  );
}

function focusKey(row: number, col: number): boolean {
  return focusCell.value?.row === row && focusCell.value.col === col;
}

function editingAt(row: number, col: number): boolean {
  return editingCell.value?.row === row && editingCell.value.col === col;
}

function focusGrid() {
  void nextTick(() => gridRef.value?.focus({ preventScroll: true }));
}

function selectCell(row: number, col: number, extend = false) {
  if (hiddenSet.value.has(col)) return;
  if (!extend || !anchor.value) anchor.value = { row, col };
  focusCell.value = { row, col };
  emit("cell-focus", { row, col });
  void nextTick(() => {
    const element = gridRef.value?.querySelector<HTMLElement>(
      `[data-cell="${row}-${col}"]`,
    );
    element?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });
}

function onCellMouseDown(row: number, col: number, event: MouseEvent) {
  if (editingAt(row, col)) return;
  if (event.button !== 0) return;
  gridRef.value?.focus({ preventScroll: true });
  if (event.shiftKey && anchor.value) {
    focusCell.value = { row, col };
    emit("cell-focus", { row, col });
    return;
  }
  anchor.value = { row, col };
  focusCell.value = { row, col };
  draggingSelection = true;
  emit("cell-focus", { row, col });
  window.addEventListener("mousemove", onSelectionDrag);
  window.addEventListener("mouseup", stopSelectionDrag);
}

function onSelectionDrag(event: MouseEvent) {
  if (!draggingSelection) return;
  const target = document.elementFromPoint(event.clientX, event.clientY);
  const cell = target?.closest<HTMLElement>("[data-cell]");
  if (!cell) return;
  const [row, col] = (cell.dataset.cell ?? "").split("-").map(Number);
  if (!Number.isFinite(row) || !Number.isFinite(col)) return;
  if (hiddenSet.value.has(col)) return;
  if (focusCell.value?.row === row && focusCell.value.col === col) return;
  focusCell.value = { row, col };
}

function stopSelectionDrag() {
  draggingSelection = false;
  window.removeEventListener("mouseup", stopSelectionDrag);
  window.removeEventListener("mousemove", onSelectionDrag);
}

function onCellClick(row: number, col: number) {
  if (editingAt(row, col)) return;
  if (!anchor.value || (anchor.value.row === row && anchor.value.col === col)) {
    selectCell(row, col);
  }
  focusGrid();
}

function openCellContextMenu(event: MouseEvent, row: number, col: number) {
  if (editingCell.value) return;
  if (!inRange(row, col)) {
    anchor.value = { row, col };
    focusCell.value = { row, col };
    emit("cell-focus", { row, col });
  }
  emit("cell-contextmenu", {
    row,
    col,
    x: event.clientX,
    y: event.clientY,
  });
}

function moveSelection(rowDelta: number, colDelta: number, extend = false) {
  const current = focusCell.value;
  if (!current) return;
  const rows = props.rows;
  const index = rowIndex(current.row);
  if (index < 0) return;
  const nextRowIndex = Math.min(rows.length - 1, Math.max(0, index + rowDelta));
  const visible = visibleCols.value;
  if (visible.length === 0) return;
  let position = visible.indexOf(current.col);
  if (position < 0) position = 0;
  position = Math.min(visible.length - 1, Math.max(0, position + colDelta));
  const nextCol = visible[position];
  selectCell(rows[nextRowIndex].row, nextCol, extend);
  focusGrid();
}

function startEdit(row: number, col: number, seed?: string) {
  if (!props.editable) return;
  if (editingAt(row, col)) return;
  anchor.value = { row, col };
  focusCell.value = { row, col };
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
    focusCell.value = { ...cell };
    const deltas: Record<MoveDirection, [number, number]> = {
      up: [-1, 0],
      down: [1, 0],
      left: [0, -1],
      right: [0, 1],
    };
    const [rowDelta, colDelta] = deltas[direction];
    moveSelection(rowDelta, colDelta);
  } else if (shouldFocus) {
    focusCell.value = { ...cell };
    focusGrid();
  }
}

function clearRange() {
  if (!props.editable) return;
  const info = rangeInfo();
  if (!info) return;
  const cells: { row: number; col: number }[] = [];
  for (let index = info.rowStart; index <= info.rowEnd; index += 1) {
    const row = props.rows[index];
    if (!row) continue;
    for (const col of visibleCols.value) {
      if (col < info.colStart || col > info.colEnd) continue;
      if (cellAt(row.row, col) === "") continue;
      cells.push({ row: row.row, col });
    }
  }
  if (cells.length) emit("range-clear", { cells });
}

function startColumnRename(index: number) {
  if (!props.columnEditable) return;
  renamingCol.value = index;
  columnDraft.value = props.headers[index] ?? "";
  void nextTick(() => {
    const input = gridRef.value?.querySelector<HTMLInputElement>(".col-head-editor");
    input?.focus();
    input?.select();
  });
}

function cancelColumnRename() {
  renamingCol.value = null;
  columnDraft.value = "";
}

function commitColumnRename() {
  const index = renamingCol.value;
  if (index === null) return;
  const value = columnDraft.value.trim();
  renamingCol.value = null;
  columnDraft.value = "";
  if (!value || value === props.headers[index]) return;
  emit("column-rename", { index, value });
}

function onColumnEditorKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    event.stopPropagation();
    commitColumnRename();
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    cancelColumnRename();
  }
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
  if (editingCell.value || renamingCol.value !== null) return;
  if (!focusCell.value) return;
  const origin = event.target as HTMLElement | null;
  if (origin?.closest("button, input, textarea, select")) return;
  const meta = event.metaKey || event.ctrlKey;

  if (meta && event.key.toLowerCase() === "d") {
    event.preventDefault();
    fillRange("down");
    return;
  }
  if (meta && event.key.toLowerCase() === "r") {
    event.preventDefault();
    fillRange("right");
    return;
  }
  if (meta && event.key.toLowerCase() === "a") {
    event.preventDefault();
    if (props.rows.length === 0 || visibleCols.value.length === 0) return;
    anchor.value = { row: props.rows[0].row, col: visibleCols.value[0] };
    focusCell.value = {
      row: props.rows[props.rows.length - 1].row,
      col: visibleCols.value[visibleCols.value.length - 1],
    };
    return;
  }
  if (event.metaKey || event.ctrlKey || event.altKey) return;

  const current = focusCell.value;
  switch (event.key) {
    case "ArrowUp":
      event.preventDefault();
      moveSelection(-1, 0, event.shiftKey);
      break;
    case "ArrowDown":
      event.preventDefault();
      moveSelection(1, 0, event.shiftKey);
      break;
    case "ArrowLeft":
      event.preventDefault();
      moveSelection(0, -1, event.shiftKey);
      break;
    case "ArrowRight":
      event.preventDefault();
      moveSelection(0, 1, event.shiftKey);
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
    case " ":
      if (event.shiftKey && props.selectableRows) {
        event.preventDefault();
        toggleRow(current.row);
      }
      break;
    case "Delete":
    case "Backspace":
      if (!props.editable) return;
      event.preventDefault();
      clearRange();
      break;
    default:
      if (props.editable && event.key.length === 1) {
        event.preventDefault();
        startEdit(current.row, current.col, event.key);
      }
  }
}

function fillRange(direction: "down" | "right") {
  if (!props.editable) return;
  const info = rangeInfo();
  if (!info) return;
  if (direction === "down" && info.rowEnd === info.rowStart) return;
  if (direction === "right" && info.colEnd === info.colStart) return;
  emit("range-fill", {
    rowStart: info.rowStart,
    rowEnd: info.rowEnd,
    colStart: info.colStart,
    colEnd: info.colEnd,
    direction,
  });
}

function onCopy(event: ClipboardEvent) {
  if (editingCell.value || renamingCol.value !== null) return;
  const info = rangeInfo();
  if (!info) return;
  const lines: string[] = [];
  for (let index = info.rowStart; index <= info.rowEnd; index += 1) {
    const row = props.rows[index];
    if (!row) continue;
    const cells: string[] = [];
    for (const col of visibleCols.value) {
      if (col < info.colStart || col > info.colEnd) continue;
      cells.push((row.cells[col] ?? "").replace(/[\t\r\n]+/g, " "));
    }
    lines.push(cells.join("\t"));
  }
  event.clipboardData?.setData("text/plain", lines.join("\n"));
  event.preventDefault();
}

function onPaste(event: ClipboardEvent) {
  if (!props.editable || !focusCell.value) return;
  if (editingCell.value || renamingCol.value !== null) return;
  const target = event.target as HTMLElement | null;
  if (target?.closest("input, textarea, select")) return;
  const text = event.clipboardData?.getData("text/plain") ?? "";
  if (!text) return;
  event.preventDefault();
  const lines = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  const values = lines.map((line) => line.split("\t"));
  if (values.length === 0) return;
  emit("range-paste", {
    row: focusCell.value.row,
    col: focusCell.value.col,
    values,
  });
}

function openColumnMenu(event: MouseEvent, index: number) {
  event.preventDefault();
  event.stopPropagation();
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuState.value = { index, x: rect.right - 4, y: rect.bottom + 4 };
}

function runColumnAction(action: string) {
  const target = menuState.value;
  menuState.value = null;
  if (!target) return;
  emit("column-action", { action, index: target.index });
}

function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (
    selectedRows.value.size > 0 &&
    !target?.closest(".select-col")
  ) {
    clearRowSelection();
  }
  if (menuState.value) return;
  if (!target || !target.closest(".ledger-grid-cell")) {
    if (!target?.closest(".ledger-grid-head")) {
      anchor.value = null;
      focusCell.value = null;
    }
  }
}

function emitSelection() {
  emit(
    "selection-change",
    [...selectedRows.value].sort((a, b) => a - b),
  );
}

function toggleRow(row: number) {
  const next = new Set(selectedRows.value);
  if (next.has(row)) next.delete(row);
  else next.add(row);
  selectedRows.value = next;
  emitSelection();
}

function clearRowSelection() {
  selectedRows.value = new Set();
  emitSelection();
}

const allSelected = computed(
  () =>
    props.rows.length > 0 && props.rows.every((row) => selectedRows.value.has(row.row)),
);

function toggleAllRows() {
  selectedRows.value = allSelected.value
    ? new Set()
    : new Set(props.rows.map((row) => row.row));
  emitSelection();
}

watch(editingCell, async (cell) => {
  if (!cell) return;
  await nextTick();
  const input = gridRef.value?.querySelector<HTMLInputElement | HTMLSelectElement>(
    ".cell-editor",
  );
  if (!input) return;
  if (input instanceof HTMLInputElement) input.focus({ preventScroll: true });
  else input.focus({ preventScroll: true });
  if (input instanceof HTMLInputElement) {
    if (seededEdit) {
      const end = input.value.length;
      input.setSelectionRange(end, end);
    } else if (input.type !== "date") {
      input.select();
    }
  }
});

watch(
  () => props.rows,
  (rows) => {
    const alive = new Set(rows.map((row) => row.row));
    if ([...selectedRows.value].some((row) => !alive.has(row))) {
      selectedRows.value = new Set([...selectedRows.value].filter((row) => alive.has(row)));
    }
    if (focusCell.value && !rows.some((row) => row.row === focusCell.value?.row)) {
      anchor.value = null;
      focusCell.value = null;
    }
  },
);

watch(
  () => props.hiddenColumns,
  () => {
    if (focusCell.value && hiddenSet.value.has(focusCell.value.col)) {
      const first = visibleCols.value[0];
      if (first !== undefined && focusCell.value) {
        focusCell.value = { row: focusCell.value.row, col: first };
      }
    }
  },
);

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
      anchor.value = null;
      focusCell.value = null;
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
  window.removeEventListener("mousemove", onSelectionDrag);
  window.removeEventListener("mouseup", stopSelectionDrag);
  window.removeEventListener("mousemove", onResizeMove);
  window.removeEventListener("mouseup", stopResize);
  document.body.classList.remove("col-resizing");
});

defineExpose({
  clearInteraction() {
    anchor.value = null;
    focusCell.value = null;
    editingCell.value = null;
    selectedRows.value = new Set();
    emitSelection();
  },
  resetWidths,
  clearRowSelection,
  beginColumnRename: startColumnRename,
});
</script>

<template>
  <div class="sheet-grid">
    <div
      ref="gridRef"
      class="table-wrap ledger-table-wrap"
      tabindex="0"
      @keydown="onGridKeydown"
      @copy="onCopy"
      @paste="onPaste"
    >
      <div class="ledger-grid" :style="{ minWidth: `${contentMinWidth}px` }">
        <div
          class="ledger-grid-head"
          :class="{ selecting: selectedRows.size > 0 }"
          :style="{ gridTemplateColumns: gridTemplate }"
        >
          <div
            v-if="selectableRows"
            class="select-col"
            :class="{ 'gutter-frozen': freezeFirstColumn }"
            :style="freezeFirstColumn ? { left: '0px' } : undefined"
          >
            <input
              class="row-check"
              type="checkbox"
              :checked="allSelected"
              title="全选当前页"
              @change="toggleAllRows"
            />
          </div>
          <div
            v-if="draggable"
            class="drag-col"
            :class="{ 'gutter-frozen': freezeFirstColumn }"
            :style="
              freezeFirstColumn
                ? { left: `${selectableRows ? SELECT_WIDTH : 0}px` }
                : undefined
            "
          ></div>
          <div
            v-for="index in visibleCols"
            :key="index"
            class="col-head"
            :class="{
              'cell-right': rightAligned(index),
              editing: renamingCol === index,
              'col-frozen': frozenCol === index,
            }"
            :style="frozenCol === index ? { left: `${frozenLeft}px` } : undefined"
            :title="columnEditable ? `${headers[index]}（双击修改列名）` : headers[index]"
            @dblclick="startColumnRename(index)"
          >
            <input
              v-if="renamingCol === index"
              v-model="columnDraft"
              class="col-head-editor"
              @keydown="onColumnEditorKeydown"
              @dblclick.stop
              @blur="commitColumnRename"
            />
            <span v-else class="th-label">{{ headers[index] }}</span>
            <button
              v-if="columnEditable"
              class="col-menu-btn"
              type="button"
              aria-label="列操作"
              @click="openColumnMenu($event, index)"
              @dblclick.stop
            >
              ⋯
            </button>
            <span
              v-if="resizable"
              class="col-resizer"
              title="拖动调整列宽，双击恢复自适应"
              @mousedown="startResize($event, index)"
              @dblclick.stop
            ></span>
          </div>
          <div class="filler-col">
            <button
              v-if="columnEditable"
              class="col-add-btn"
              type="button"
              title="添加列"
              aria-label="添加列"
              @click.stop="emit('column-add')"
            >
              <svg viewBox="0 0 16 16" aria-hidden="true">
                <path d="M8 3v10M3 8h10" />
              </svg>
            </button>
          </div>
          <div v-if="hasActions" class="actions-col">操作</div>
        </div>

        <div ref="tbodyRef" class="ledger-grid-body">
          <template
            v-for="item in displayItems"
            :key="item.kind === 'group' ? item.path : item.row.row"
          >
            <div
              v-if="item.kind === 'group'"
              class="ledger-grid-group"
              :style="{ paddingLeft: `${14 + item.depth * 18}px` }"
            >
              <button
                class="group-toggle"
                type="button"
                :aria-label="collapsedGroups.has(item.path) ? `展开分组 ${item.name}` : `折叠分组 ${item.name}`"
                :aria-expanded="!collapsedGroups.has(item.path)"
                @click="toggleGroup(item.path)"
              >
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <path
                    :d="
                      collapsedGroups.has(item.path)
                        ? 'M6 4l4 4-4 4'
                        : 'M4 6l4 4 4-4'
                    "
                  />
                </svg>
              </button>
              <span class="group-name">{{ item.name }}</span>
              <span class="group-count">{{ item.count }} 行</span>
            </div>
            <div
              v-else
              class="ledger-grid-row"
              :class="{ 'row-selected': selectedRows.has(item.row.row) }"
              :style="{ gridTemplateColumns: gridTemplate }"
            >
              <div
                v-if="selectableRows"
                class="select-col"
                :class="{ 'gutter-frozen': freezeFirstColumn }"
                :style="freezeFirstColumn ? { left: '0px' } : undefined"
              >
                <input
                  class="row-check"
                  type="checkbox"
                  :checked="selectedRows.has(item.row.row)"
                  @change="toggleRow(item.row.row)"
                />
              </div>
              <div
                v-if="draggable"
                class="drag-col"
                :class="{ 'gutter-frozen': freezeFirstColumn }"
                :style="
                  freezeFirstColumn
                    ? { left: `${selectableRows ? SELECT_WIDTH : 0}px` }
                    : undefined
                "
              >
                <span
                  class="drag-handle"
                  :class="{ disabled: dragDisabled }"
                  :title="dragDisabled ? '当前状态下不能调整顺序' : '拖动调整顺序'"
                >
                  ⠿
                </span>
              </div>
              <div
                v-for="index in visibleCols"
                :key="index"
                class="ledger-grid-cell"
                :class="{
                  'cell-selected': focusKey(item.row.row, index),
                  'cell-in-range':
                    inRange(item.row.row, index) && !focusKey(item.row.row, index),
                  'cell-editing': editingAt(item.row.row, index),
                  'cell-right': rightAligned(index),
                  'cell-frozen': frozenCol === index,
                }"
                :style="frozenCol === index ? { left: `${frozenLeft}px` } : undefined"
                :data-cell="`${item.row.row}-${index}`"
                :title="item.row.cells[index] ?? ''"
                @mousedown="onCellMouseDown(item.row.row, index, $event)"
                @click="onCellClick(item.row.row, index)"
                @dblclick="startEdit(item.row.row, index)"
                @contextmenu.prevent="
                  openCellContextMenu($event, item.row.row, index)
                "
              >
                <select
                  v-if="editingAt(item.row.row, index) && formatOf(index) === 'select'"
                  v-model="cellDraft"
                  class="cell-editor"
                  @change="commitCell()"
                  @blur="commitCell(undefined, false)"
                  @keydown="onEditorKeydown"
                >
                  <option value="">（空）</option>
                  <option v-for="option in optionsOf(index)" :key="option" :value="option">
                    {{ option }}
                  </option>
                </select>
                <input
                  v-else-if="editingAt(item.row.row, index)"
                  v-model="cellDraft"
                  class="cell-editor"
                  :type="formatOf(index) === 'date' ? 'date' : 'text'"
                  @keydown="onEditorKeydown"
                  @blur="commitCell(undefined, false)"
                />
                <slot
                  v-else
                  name="cell"
                  :row="item.row"
                  :col="index"
                  :value="item.row.cells[index] ?? ''"
                >
                  <span
                    v-if="formatOf(index) === 'multi_select'"
                    class="cell-tags"
                  >
                    <span
                      v-for="tag in splitTags(item.row.cells[index] ?? '')"
                      :key="tag"
                      class="cell-tag"
                    >
                      {{ tag }}
                    </span>
                  </span>
                  <span
                    v-else-if="formatOf(index) === 'select' && item.row.cells[index]"
                    class="cell-tag"
                  >
                    {{ item.row.cells[index] }}
                  </span>
                  <span v-else class="cell-text">{{ item.row.cells[index] ?? "" }}</span>
                </slot>
              </div>
              <div class="filler-col"></div>
              <div v-if="hasActions" class="actions-col">
                <div class="row-actions">
                  <slot name="row-actions" :row="item.row" />
                </div>
              </div>
            </div>
          </template>
        </div>

      </div>
      <div v-if="loading" class="table-empty">加载中</div>
      <div v-else-if="rows.length === 0" class="table-empty">{{ emptyText }}</div>
    </div>
  </div>

  <Teleport to="body">
    <div v-if="menuState" class="col-menu-backdrop" @click="menuState = null">
      <div
        class="col-menu"
        :style="{ left: `${menuState.x}px`, top: `${menuState.y}px` }"
        @click.stop
      >
        <div class="col-menu-title">{{ headers[menuState.index] || "列" }}</div>
        <button class="col-menu-item" @click="runColumnAction('rename')">重命名</button>
        <button class="col-menu-item" @click="runColumnAction('edit')">编辑列…</button>
        <div class="col-menu-sep"></div>
        <button class="col-menu-item" @click="runColumnAction('insert-left')">
          左侧插入列
        </button>
        <button class="col-menu-item" @click="runColumnAction('insert-right')">
          右侧插入列
        </button>
        <button class="col-menu-item" @click="runColumnAction('duplicate')">复制列</button>
        <button class="col-menu-item" @click="runColumnAction('hide')">隐藏列</button>
        <div class="col-menu-sep"></div>
        <button class="col-menu-item" @click="runColumnAction('sort-asc')">升序</button>
        <button class="col-menu-item" @click="runColumnAction('sort-desc')">降序</button>
        <button class="col-menu-item" @click="runColumnAction('clear-sort')">
          清除排序
        </button>
        <button class="col-menu-item" @click="runColumnAction('filter')">筛选…</button>
        <div class="col-menu-sep"></div>
        <button class="col-menu-item danger" @click="runColumnAction('delete')">
          删除列
        </button>
      </div>
    </div>
  </Teleport>
</template>

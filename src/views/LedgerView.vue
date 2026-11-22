<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import Sortable from "sortablejs";
import { api, type SheetColumnPayload } from "../api";
import DropdownSelect from "../components/DropdownSelect.vue";
import SheetGrid from "../components/SheetGrid.vue";
import SheetTabs from "../components/SheetTabs.vue";
import ViewRulesBar from "../components/ViewRulesBar.vue";
import { notify } from "../toast";
import type {
  ColumnFormat,
  SheetInfo,
  SheetRowData,
  SheetTable,
  TableViewConfig,
  TableViewFilter,
  Template,
} from "../types";

const PAGE_SIZE = 100;

const templates = ref<Template[]>([]);
const templateId = ref("");
const templateOptions = computed(() =>
  templates.value.map((item) => ({ value: item.id, label: item.name })),
);
const table = ref<SheetTable | null>(null);
const loading = ref(false);
const keyword = ref("");
const page = ref(1);
const selectedRowIds = ref<number[]>([]);
const cellMenu = ref<{ row: number; col: number; x: number; y: number } | null>(
  null,
);
const cellMenuStyle = computed(() => {
  const menu = cellMenu.value;
  if (!menu) return {};
  return {
    left: `${Math.max(8, Math.min(menu.x, window.innerWidth - 196))}px`,
    top: `${Math.max(8, Math.min(menu.y, window.innerHeight - 392))}px`,
  };
});
const dataSheetList = ref<SheetInfo[]>([]);
const activeDataSheet = ref("");
const addingDataSheet = ref(false);

interface SheetGridApi {
  clearInteraction: () => void;
  resetWidths: () => void;
  clearRowSelection: () => void;
  beginColumnRename: (index: number) => void;
}
const gridRef = ref<SheetGridApi | null>(null);
const widthCacheKey = computed(() =>
  templateId.value && activeDataSheet.value
    ? `jy-ledger-widths:${templateId.value}:${activeDataSheet.value}`
    : templateId.value
      ? `jy-ledger-widths:${templateId.value}`
      : "",
);

interface ColumnDraft {
  uid: number;
  source: number;
  label: string;
  hint: string;
  width: number;
  format: ColumnFormat;
  options: string[];
}

const columnEditorOpen = ref(false);
const columnDrafts = ref<ColumnDraft[]>([]);
const columnBusy = ref(false);
const columnOpBusy = ref(false);
const columnListRef = ref<HTMLElement | null>(null);
let columnSortable: Sortable | null = null;
let columnSeq = 0;

const editingRow = ref<number | null>(null);
const draftValues = ref<string[]>([]);
const originValues = ref<string[]>([]);
const saving = ref(false);
const addingRow = ref(false);
const deleteConfirmRow = ref<number | null>(null);
let deleteTimer: number | undefined;

// 所有写表格的操作排队执行，避免同一格的连续修改乱序落盘。
let writeChain: Promise<void> = Promise.resolve();

function queueWrite<T>(task: () => Promise<T>): Promise<T> {
  const run = writeChain.then(task, task);
  writeChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

const headers = computed(() => table.value?.headers ?? []);

// 标准名录：与数据页共用同一套表格状态和交互，仅页面标题不同。
const whitelistOpen = ref(false);

interface ColumnMeta {
  /** 这一列在调整前的 Excel 列号；0 表示新增列。 */
  source: number;
  label: string;
  hint: string;
  width: number;
  format: ColumnFormat;
  options: string[];
}

function columnMetasFor(sheetName: string, currentHeaders: string[]): ColumnMeta[] {
  const template = templates.value.find((item) => item.id === templateId.value);
  const sheetMeta = template?.sheets.find((sheet) => sheet.name === sheetName);
  const source = sheetMeta?.columns ?? template?.columns ?? [];
  return currentHeaders.map((label, index) => {
    const meta = source[index];
    return {
      source: index + 1,
      label,
      hint: meta?.hint ?? "",
      width: meta?.width ?? 18,
      format: (meta?.format as ColumnFormat) ?? "text",
      options: meta?.options ?? [],
    };
  });
}

const dataMetas = computed(() =>
  columnMetasFor(activeDataSheet.value, headers.value),
);
const dataFormats = computed(() => dataMetas.value.map((meta) => meta.format));
const dataOptions = computed(() => dataMetas.value.map((meta) => meta.options));
const dataKeys = computed(() => table.value?.keys ?? []);

const viewConfig = ref<TableViewConfig>({});
const filterEditor = ref<{
  index: number;
  operator: TableViewFilter["operator"];
  value: string;
} | null>(null);
const columnSettings = ref<{
  index: number;
  label: string;
  format: ColumnFormat;
  options: string;
  hint: string;
} | null>(null);
const replaceOpen = ref(false);
const replaceFind = ref("");
const replaceWith = ref("");
const replaceBusy = ref(false);

const viewStorageKey = computed(() =>
  templateId.value && activeDataSheet.value
    ? `jy-table-view:${templateId.value}:${activeDataSheet.value}`
    : "",
);

function readStoredConfig(key: string): TableViewConfig {
  if (!key) return {};
  try {
    const parsed = JSON.parse(
      localStorage.getItem(key) ?? "{}",
    ) as TableViewConfig;
    if (!parsed.sorts && parsed.sort) parsed.sorts = [parsed.sort];
    if (!parsed.groupKeys && parsed.groupKey) {
      parsed.groupKeys = [{ key: parsed.groupKey, direction: "asc" }];
    }
    delete parsed.sort;
    delete parsed.groupKey;
    return parsed;
  } catch {
    return {};
  }
}

function writeStoredConfig(key: string, config: TableViewConfig) {
  if (!key) return;
  try {
    localStorage.setItem(key, JSON.stringify(config));
  } catch {
    // 本地缓存不可用时忽略，配置只在本次运行内有效。
  }
}

function keyIndex(keys: string[], key: string): number {
  return keys.indexOf(key);
}

function filterAndSortRows(
  rows: SheetRowData[],
  keys: string[],
  config: TableViewConfig,
  query: string,
): SheetRowData[] {
  let result = rows;
  const text = query.trim().toLowerCase();
  if (text) {
    result = result.filter((row) =>
      row.cells.some((cell) => cell.toLowerCase().includes(text)),
    );
  }
  const filters = config.filters ?? [];
  if (filters.length) {
    const matchAll = (config.filterLogic ?? "and") !== "or";
    result = result.filter((row) => {
      const matches = filters.map((filter) => {
        const index = keyIndex(keys, filter.key);
        if (index < 0) return true;
        const value = (row.cells[index] ?? "").trim();
        const target = filter.value.trim();
        switch (filter.operator) {
          case "contains":
            return target === "" || value.toLowerCase().includes(target.toLowerCase());
          case "equals":
            return value.toLowerCase() === target.toLowerCase();
          case "empty":
            return value === "";
          case "notEmpty":
            return value !== "";
          case "gt":
            return Number.isFinite(Number(value)) && Number(value) > Number(target);
          case "lt":
            return Number.isFinite(Number(value)) && Number(value) < Number(target);
          default:
            return true;
        }
      });
      return matchAll ? matches.every(Boolean) : matches.some(Boolean);
    });
  }
  const sorts = config.sorts ?? [];
  if (sorts.length) {
    result = [...result].sort((a, b) => {
      for (const sort of sorts) {
        const index = keyIndex(keys, sort.key);
        if (index < 0) continue;
        const left = (a.cells[index] ?? "").trim();
        const right = (b.cells[index] ?? "").trim();
        const leftNumber = Number(left);
        const rightNumber = Number(right);
        const comparison =
          left !== "" &&
          right !== "" &&
          Number.isFinite(leftNumber) &&
          Number.isFinite(rightNumber)
            ? leftNumber - rightNumber
            : left.localeCompare(right, "zh-CN");
        if (comparison !== 0) {
          return sort.direction === "asc" ? comparison : -comparison;
        }
      }
      return 0;
    });
  }
  return result;
}

function visibleColumnIndices(config: TableViewConfig, keys: string[]): number[] {
  const hidden = new Set(config.hiddenKeys ?? []);
  return keys
    .map((key, index) => ({ key, index }))
    .filter((item) => !hidden.has(item.key))
    .map((item) => item.index);
}

const dataHiddenColumns = computed(() =>
  (viewConfig.value.hiddenKeys ?? [])
    .map((key) => keyIndex(dataKeys.value, key))
    .filter((index) => index >= 0),
);
const dataGroupColumns = computed(() =>
  (viewConfig.value.groupKeys ?? [])
    .map((group) => keyIndex(dataKeys.value, group.key))
    .filter((index) => index >= 0),
);
const dataGroupDirections = computed(() =>
  (viewConfig.value.groupKeys ?? [])
    .filter((group) => keyIndex(dataKeys.value, group.key) >= 0)
    .map((group) => group.direction),
);
const dataSorts = computed(() => viewConfig.value.sorts ?? []);
const dataFilters = computed(() => viewConfig.value.filters ?? []);
const dataReorderDisabled = computed(
  () =>
    keyword.value.trim() !== "" ||
    dataSorts.value.length > 0 ||
    dataFilters.value.length > 0 ||
    dataGroupColumns.value.length > 0,
);
const dataViewRulesActive = computed(
  () => dataSorts.value.length > 0 || dataFilters.value.length > 0,
);

interface HistoryEntry {
  undo: () => Promise<void>;
  redo: () => Promise<void>;
}

const undoStack: HistoryEntry[] = [];
const redoStack: HistoryEntry[] = [];
const historyVersion = ref(0);

function pushHistory(entry: HistoryEntry) {
  undoStack.push(entry);
  if (undoStack.length > 100) undoStack.shift();
  redoStack.length = 0;
  historyVersion.value += 1;
}

function clearHistory() {
  undoStack.length = 0;
  redoStack.length = 0;
  historyVersion.value += 1;
}

async function undoAction() {
  const entry = undoStack.pop();
  if (!entry) return;
  try {
    await entry.undo();
    redoStack.push(entry);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    historyVersion.value += 1;
  }
}

async function redoAction() {
  const entry = redoStack.pop();
  if (!entry) return;
  try {
    await entry.redo();
    undoStack.push(entry);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    historyVersion.value += 1;
  }
}

function onGlobalKeydown(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null;
  if (target?.closest("input, textarea, select, [contenteditable='true']")) return;
  if (document.querySelector(".modal-backdrop, .drawer-backdrop")) return;
  const meta = event.metaKey || event.ctrlKey;
  if (!meta) return;
  const key = event.key.toLowerCase();
  if (key === "z") {
    event.preventDefault();
    if (event.shiftKey) void redoAction();
    else void undoAction();
    return;
  }
  if (key === "y") {
    event.preventDefault();
    void redoAction();
  }
}

function openWhitelist() {
  whitelistOpen.value = true;
}

const drawerTitle = computed(() => `编辑第 ${editingRow.value} 行`);
const subtitle = computed(() => {
  if (!table.value) return loading.value ? "读取中…" : "表格尚未生成";
  return `${table.value.sheet} · ${table.value.rows.length} 行 · ${table.value.path}`;
});

const filteredRows = computed(() =>
  filterAndSortRows(
    table.value?.rows ?? [],
    dataKeys.value,
    viewConfig.value,
    keyword.value,
  ),
);

const totalPages = computed(() =>
  Math.max(1, Math.ceil(filteredRows.value.length / PAGE_SIZE)),
);

const visibleRows = computed(() => {
  if (dataGroupColumns.value.length > 0) return filteredRows.value;
  const start = (page.value - 1) * PAGE_SIZE;
  return filteredRows.value.slice(start, start + PAGE_SIZE);
});

function openColumnEditor() {
  if (headers.value.length === 0) {
    notify("请先在设置里配置导出列", "error");
    return;
  }
  const template = templates.value.find((item) => item.id === templateId.value);
  const sheetMeta = template?.sheets.find(
    (sheet) => sheet.name === activeDataSheet.value,
  );
  columnDrafts.value = headers.value.map((label, index) => {
    const meta = sheetMeta?.columns[index] ?? template?.columns[index];
    return {
      uid: ++columnSeq,
      source: index + 1,
      label,
      hint: meta?.hint ?? "",
      width: meta?.width ?? 18,
      format: (meta?.format as ColumnFormat) ?? "text",
      options: meta?.options ?? [],
    };
  });
  columnEditorOpen.value = true;
}

function closeColumnEditor() {
  columnEditorOpen.value = false;
  columnDrafts.value = [];
  columnSortable?.destroy();
  columnSortable = null;
}

function addColumnDraft() {
  columnDrafts.value.push({
    uid: ++columnSeq,
    source: 0,
    label: "",
    hint: "",
    width: 18,
    format: "text",
    options: [],
  });
}

function removeColumnDraft(uid: number) {
  if (columnDrafts.value.length <= 1) {
    notify("至少保留一列", "error");
    return;
  }
  columnDrafts.value = columnDrafts.value.filter((draft) => draft.uid !== uid);
}

function initColumnSortable() {
  columnSortable?.destroy();
  columnSortable = null;
  const el = columnListRef.value;
  if (!el) return;
  columnSortable = Sortable.create(el, {
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
    },
    onEnd(event) {
      el.classList.remove("is-sorting");
      window.getSelection()?.removeAllRanges();
      const { oldIndex, newIndex } = event;
      if (oldIndex == null || newIndex == null || oldIndex === newIndex) return;
      const [moved] = columnDrafts.value.splice(oldIndex, 1);
      if (moved) columnDrafts.value.splice(newIndex, 0, moved);
    },
  });
}

watch(columnListRef, initColumnSortable, { flush: "post" });

async function saveColumnSettings() {
  if (columnBusy.value) return;
  const drafts = columnDrafts.value;
  if (drafts.length === 0) {
    notify("至少保留一列", "error");
    return;
  }
  const labels = drafts.map((draft) => draft.label.trim());
  if (labels.some((label) => !label)) {
    notify("列名不能为空", "error");
    return;
  }
  const payload = drafts.map((draft) => ({
    source: draft.source,
    label: draft.label.trim(),
    hint: draft.hint.trim(),
    width: Math.min(80, Math.max(6, Number(draft.width) || 18)),
    format: draft.format,
    options: draft.options,
  }));
  columnBusy.value = true;
  try {
    await queueWrite(() =>
      api.saveSheetColumns(templateId.value, activeDataSheet.value, payload),
    );
    await refreshTemplates();
    gridRef.value?.resetWidths();
    clearHistory();
    await reload(false);
    closeColumnEditor();
    notify("列设置已保存");
  } catch (error) {
    notify(String(error), "error");
  } finally {
    columnBusy.value = false;
  }
}

async function refreshTemplates() {
  templates.value = await api.listTemplates();
}

function nextColumnLabel(labels: string[]): string {
  if (!labels.includes("新列")) return "新列";
  let index = 2;
  while (labels.includes(`新列${index}`)) index += 1;
  return `新列${index}`;
}

function newColumnMeta(labels: string[]): ColumnMeta {
  return {
    source: 0,
    label: nextColumnLabel(labels),
    hint: "",
    width: 18,
    format: "text",
    options: [],
  };
}

function columnPayloadFromMetas(metas: ColumnMeta[]): SheetColumnPayload[] {
  return metas.map((meta) => ({
    source: meta.source,
    label: meta.label,
    hint: meta.hint,
    width: meta.width,
    format: meta.format,
    options: meta.options,
  }));
}

async function applyColumns(
  sheetName: string,
  columns: SheetColumnPayload[],
  after: () => Promise<void>,
): Promise<boolean> {
  if (!templateId.value || !sheetName || columnOpBusy.value) return false;
  columnOpBusy.value = true;
  try {
    await queueWrite(() => api.saveSheetColumns(templateId.value, sheetName, columns));
    await refreshTemplates();
    gridRef.value?.resetWidths();
    clearHistory();
    await after();
    return true;
  } catch (error) {
    notify(String(error), "error");
    return false;
  } finally {
    columnOpBusy.value = false;
  }
}

async function saveColumnMetas(
  sheetName: string,
  metas: ColumnMeta[],
  after: () => Promise<void>,
): Promise<boolean> {
  return applyColumns(sheetName, columnPayloadFromMetas(metas), after);
}

async function renameColumnOn(
  sheetName: string,
  current: ColumnMeta[],
  index: number,
  value: string,
  after: () => Promise<void>,
) {
  const label = value.trim();
  if (!label) {
    notify("列名不能为空", "error");
    return;
  }
  if (current[index]?.label === label) return;
  const metas = current.slice();
  metas[index] = { ...metas[index], label };
  await saveColumnMetas(sheetName, metas, after);
}

async function addColumnOn(
  sheetName: string,
  current: ColumnMeta[],
  after: () => Promise<void>,
) {
  const metas = current.slice();
  metas.push(newColumnMeta(metas.map((meta) => meta.label)));
  await saveColumnMetas(sheetName, metas, after);
}

async function insertColumnOn(
  sheetName: string,
  current: ColumnMeta[],
  index: number,
  side: "left" | "right",
  after: () => Promise<void>,
) {
  const metas = current.slice();
  metas.splice(
    side === "left" ? index : index + 1,
    0,
    newColumnMeta(metas.map((meta) => meta.label)),
  );
  await saveColumnMetas(sheetName, metas, after);
}

async function duplicateColumnOn(
  sheetName: string,
  current: ColumnMeta[],
  index: number,
  after: () => Promise<void>,
) {
  const source = current[index];
  if (!source) return;
  const metas = current.slice();
  metas.splice(index + 1, 0, {
    ...source,
    source: 0,
    options: [...source.options],
  });
  await saveColumnMetas(sheetName, metas, after);
}

async function deleteColumnOn(
  sheetName: string,
  current: ColumnMeta[],
  index: number,
  after: () => Promise<void>,
) {
  if (current.length <= 1) {
    notify("至少保留一列", "error");
    return;
  }
  const metas = current.slice();
  metas.splice(index, 1);
  await saveColumnMetas(sheetName, metas, after);
}

function renameDataColumn(payload: { index: number; value: string }) {
  void renameColumnOn(
    activeDataSheet.value,
    dataMetas.value,
    payload.index,
    payload.value,
    () => reload(false),
  );
}

function addDataColumn() {
  void addColumnOn(activeDataSheet.value, dataMetas.value, () => reload(false));
}

function buildRowUpdates(changes: { row: number; col: number; value: string }[]) {
  const grouped = new Map<number, Map<number, string>>();
  for (const change of changes) {
    let row = grouped.get(change.row);
    if (!row) {
      row = new Map();
      grouped.set(change.row, row);
    }
    row.set(change.col, change.value);
  }
  return [...grouped.entries()].map(([row, cells]) => ({
    row,
    values: headers.value.map((_, index) =>
      cells.has(index) ? (cells.get(index) ?? "") : null,
    ),
  }));
}

async function writeCells(
  changes: { row: number; col: number; value: string }[],
) {
  if (changes.length === 0) return;
  await queueWrite(() =>
    api.updateSheetRows(
      templateId.value,
      activeDataSheet.value,
      buildRowUpdates(changes),
    ),
  );
}

async function writeCellsWithHistory(
  changes: { row: number; col: number; before: string; after: string }[],
): Promise<boolean> {
  if (changes.length === 0) return true;
  for (const change of changes) {
    const target = table.value?.rows.find((row) => row.row === change.row);
    if (target) target.cells[change.col] = change.after;
  }
  pushHistory({
    undo: async () => {
      await writeCells(
        changes.map((change) => ({
          row: change.row,
          col: change.col,
          value: change.before,
        })),
      );
      await reload(false);
    },
    redo: async () => {
      await writeCells(
        changes.map((change) => ({
          row: change.row,
          col: change.col,
          value: change.after,
        })),
      );
      await reload(false);
    },
  });
  try {
    await writeCells(
      changes.map((change) => ({
        row: change.row,
        col: change.col,
        value: change.after,
      })),
    );
    return true;
  } catch (error) {
    await reload(false);
    notify(String(error), "error");
    return false;
  }
}

function onRangePaste(payload: {
  row: number;
  col: number;
  values: string[][];
}) {
  const rows = visibleRows.value;
  let start = rows.findIndex((row) => row.row === payload.row);
  if (start < 0) start = 0;
  const visible = visibleColumnIndices(viewConfig.value, dataKeys.value);
  let startCol = visible.indexOf(payload.col);
  if (startCol < 0) startCol = 0;
  const changes: { row: number; col: number; before: string; after: string }[] = [];
  payload.values.forEach((line, rowOffset) => {
    const row = rows[start + rowOffset];
    if (!row) return;
    line.forEach((value, cellOffset) => {
      const col = visible[startCol + cellOffset];
      if (col === undefined) return;
      const before = row.cells[col] ?? "";
      if (before !== value) changes.push({ row: row.row, col, before, after: value });
    });
  });
  void writeCellsWithHistory(changes).then((ok) => {
    if (ok && dataViewRulesActive.value) void reload(false);
  });
}

function onRangeClear(payload: { cells: { row: number; col: number }[] }) {
  const changes = payload.cells
    .map((cell) => {
      const before =
        table.value?.rows.find((row) => row.row === cell.row)?.cells[cell.col] ?? "";
      return { row: cell.row, col: cell.col, before, after: "" };
    })
    .filter((change) => change.before !== "");
  void writeCellsWithHistory(changes).then((ok) => {
    if (ok && dataViewRulesActive.value) void reload(false);
  });
}

function onRangeFill(payload: {
  rowStart: number;
  rowEnd: number;
  colStart: number;
  colEnd: number;
  direction: "down" | "right";
}) {
  const rows = visibleRows.value;
  const visible = visibleColumnIndices(viewConfig.value, dataKeys.value);
  const cols = visible.filter(
    (col) => col >= payload.colStart && col <= payload.colEnd,
  );
  const changes: { row: number; col: number; before: string; after: string }[] = [];
  if (payload.direction === "down") {
    const source = rows[payload.rowStart];
    if (!source) return;
    for (let index = payload.rowStart + 1; index <= payload.rowEnd; index += 1) {
      const row = rows[index];
      if (!row) continue;
      for (const col of cols) {
        const before = row.cells[col] ?? "";
        const after = source.cells[col] ?? "";
        if (before !== after) changes.push({ row: row.row, col, before, after });
      }
    }
  } else {
    const sourceCol = cols[0];
    if (sourceCol === undefined) return;
    for (let index = payload.rowStart; index <= payload.rowEnd; index += 1) {
      const row = rows[index];
      if (!row) continue;
      const after = row.cells[sourceCol] ?? "";
      for (const col of cols) {
        if (col <= sourceCol) continue;
        const before = row.cells[col] ?? "";
        if (before !== after) changes.push({ row: row.row, col, before, after });
      }
    }
  }
  void writeCellsWithHistory(changes).then((ok) => {
    if (ok && dataViewRulesActive.value) void reload(false);
  });
}

function persistViewConfig() {
  writeStoredConfig(viewStorageKey.value, viewConfig.value);
}

function applyViewConfig(patch: Partial<TableViewConfig>) {
  viewConfig.value = { ...viewConfig.value, ...patch };
  page.value = 1;
  gridRef.value?.clearInteraction();
  persistViewConfig();
}

function toggleFreezeColumn() {
  applyViewConfig({ freezeFirstColumn: !viewConfig.value.freezeFirstColumn });
}

function showAllColumns() {
  applyViewConfig({ hiddenKeys: [] });
}

function resetColumnWidths() {
  gridRef.value?.resetWidths();
}

function clearViewRules() {
  applyViewConfig({
    filters: [],
    sorts: [],
    groupKeys: [],
    hiddenKeys: [],
    freezeFirstColumn: false,
  });
}

function openCellMenu(payload: {
  row: number;
  col: number;
  x: number;
  y: number;
}) {
  if (!selectedRowIds.value.includes(payload.row)) {
    gridRef.value?.clearRowSelection();
    selectedRowIds.value = [];
  }
  cellMenu.value = payload;
}

function closeCellMenu() {
  cellMenu.value = null;
}

function menuRowIds(): number[] {
  const menu = cellMenu.value;
  if (!menu) return [];
  if (selectedRowIds.value.includes(menu.row)) return [...selectedRowIds.value];
  return [menu.row];
}

async function copyMenuCell() {
  const menu = cellMenu.value;
  if (!menu) return;
  const value =
    table.value?.rows.find((row) => row.row === menu.row)?.cells[menu.col] ?? "";
  closeCellMenu();
  try {
    await navigator.clipboard.writeText(value);
    notify("已复制单元格内容");
  } catch {
    notify("复制失败，请使用复制快捷键", "error");
  }
}

async function duplicateMenuRows() {
  const ids = menuRowIds();
  closeCellMenu();
  await duplicateRows(ids);
}

async function clearMenuRows() {
  const ids = menuRowIds();
  closeCellMenu();
  await clearRows(ids);
}

async function deleteMenuRows() {
  const ids = menuRowIds();
  closeCellMenu();
  await deleteRows(ids);
}

function replaceFromMenu() {
  closeCellMenu();
  replaceOpen.value = true;
}

function columnSettingsFromMenu() {
  closeCellMenu();
  openColumnEditor();
}

function freezeFromMenu() {
  closeCellMenu();
  toggleFreezeColumn();
}

function showAllColumnsFromMenu() {
  closeCellMenu();
  showAllColumns();
}

function resetWidthsFromMenu() {
  closeCellMenu();
  resetColumnWidths();
}

function clearViewRulesFromMenu() {
  closeCellMenu();
  clearViewRules();
}

const filterOperatorOptions = [
  { value: "contains", label: "包含" },
  { value: "equals", label: "等于" },
  { value: "empty", label: "为空" },
  { value: "notEmpty", label: "不为空" },
  { value: "gt", label: "大于" },
  { value: "lt", label: "小于" },
];

const columnFormatOptions = [
  { value: "text", label: "文本" },
  { value: "number", label: "数字" },
  { value: "date", label: "日期" },
  { value: "select", label: "单选" },
  { value: "multi_select", label: "多选" },
];

function onFilterOperatorChange(value: string) {
  if (!filterEditor.value) return;
  filterEditor.value.operator = value as TableViewFilter["operator"];
}

function onColumnFormatChange(value: string) {
  if (!columnSettings.value) return;
  columnSettings.value.format = value as ColumnFormat;
}

function openFilter(index: number) {
  const key = dataKeys.value[index] ?? `col_${index + 1}`;
  const existing = (viewConfig.value.filters ?? []).find(
    (filter) => filter.key === key,
  );
  filterEditor.value = {
    index,
    operator: existing?.operator ?? "contains",
    value: existing?.value ?? "",
  };
}

function saveFilter() {
  const editor = filterEditor.value;
  if (!editor) return;
  const key = dataKeys.value[editor.index] ?? `col_${editor.index + 1}`;
  const others = (viewConfig.value.filters ?? []).filter(
    (filter) => filter.key !== key,
  );
  const emptyValue = editor.value.trim() === "";
  const filters =
    emptyValue && editor.operator !== "empty" && editor.operator !== "notEmpty"
      ? others
      : [
          ...others,
          {
            key,
            operator: editor.operator,
            value: editor.value.trim(),
          },
        ];
  filterEditor.value = null;
  applyViewConfig({ filters });
}

function filterColumnLabel(): string {
  const editor = filterEditor.value;
  if (!editor) return "列";
  return headers.value[editor.index] || "列";
}

function closeWhitelist() {
  whitelistOpen.value = false;
}

function onDataColumnAction(payload: { action: string; index: number }) {
  if (!table.value) return;
  const index = payload.index;
  const key = dataKeys.value[index];
  switch (payload.action) {
    case "rename":
      gridRef.value?.beginColumnRename(index);
      break;
    case "edit":
      openColumnSetting(index);
      break;
    case "insert-left":
      void insertColumnOn(
        activeDataSheet.value,
        dataMetas.value,
        index,
        "left",
        () => reload(false),
      );
      break;
    case "insert-right":
      void insertColumnOn(
        activeDataSheet.value,
        dataMetas.value,
        index,
        "right",
        () => reload(false),
      );
      break;
    case "duplicate":
      void duplicateColumnOn(
        activeDataSheet.value,
        dataMetas.value,
        index,
        () => reload(false),
      );
      break;
    case "hide":
      if (key) {
        const hidden = new Set(viewConfig.value.hiddenKeys ?? []);
        hidden.add(key);
        applyViewConfig({ hiddenKeys: [...hidden] });
      }
      break;
    case "sort-asc":
      if (key) applyViewConfig({ sorts: [{ key, direction: "asc" }] });
      break;
    case "sort-desc":
      if (key) applyViewConfig({ sorts: [{ key, direction: "desc" }] });
      break;
    case "clear-sort":
      applyViewConfig({ sorts: [] });
      break;
    case "filter":
      openFilter(index);
      break;
    case "delete":
      void deleteColumnOn(
        activeDataSheet.value,
        dataMetas.value,
        index,
        () => reload(false),
      );
      break;
  }
}

function openColumnSetting(index: number) {
  const meta = dataMetas.value[index];
  if (!meta) return;
  columnSettings.value = {
    index,
    label: meta.label,
    format: meta.format,
    options: meta.options.join("，"),
    hint: meta.hint,
  };
}

async function saveColumnSetting() {
  const editor = columnSettings.value;
  if (!editor) return;
  const metas = dataMetas.value.slice();
  metas[editor.index] = {
    ...metas[editor.index],
    label: editor.label.trim() || metas[editor.index].label,
    format: editor.format,
    options: editor.options
      .split(/[,，]/)
      .map((option) => option.trim())
      .filter(Boolean),
    hint: editor.hint.trim(),
  };
  columnSettings.value = null;
  await saveColumnMetas(activeDataSheet.value, metas, () => reload(false));
}

async function runReplace() {
  if (!table.value || replaceBusy.value) return;
  const find = replaceFind.value;
  if (!find) return;
  const changes: { row: number; col: number; before: string; after: string }[] = [];
  for (const row of table.value.rows) {
    row.cells.forEach((cell, col) => {
      if (!cell.includes(find)) return;
      changes.push({
        row: row.row,
        col,
        before: cell,
        after: cell.split(find).join(replaceWith.value),
      });
    });
  }
  if (changes.length === 0) {
    notify("没有找到匹配内容");
    return;
  }
  replaceBusy.value = true;
  try {
    const ok = await writeCellsWithHistory(changes);
    if (ok) {
      if (dataViewRulesActive.value) await reload(false);
      replaceOpen.value = false;
      notify(`已替换 ${changes.length} 处`);
    }
  } finally {
    replaceBusy.value = false;
  }
}

async function reorderRows(oldIndex: number, newIndex: number) {  if (dataReorderDisabled.value) return;
  const rows = table.value?.rows;
  if (!rows) return;
  const base = (page.value - 1) * PAGE_SIZE;
  const from = base + oldIndex;
  const to = base + newIndex;
  if (from < 0 || from >= rows.length || to < 0 || to >= rows.length) return;

  const order = rows.map((row) => row.row);
  const [moved] = order.splice(from, 1);
  if (moved == null) return;
  order.splice(to, 0, moved);

  const cellsByRow = new Map(rows.map((row) => [row.row, [...row.cells]]));
  rows.forEach((row, index) => {
    row.cells = cellsByRow.get(order[index]) ?? [];
  });
  try {
    await queueWrite(() =>
      api.reorderSheetRows(templateId.value, activeDataSheet.value, order),
    );
    clearHistory();
  } catch (error) {
    notify(String(error), "error");
    await reload(false);
  }
}

function onRowsReorder(payload: { oldIndex: number; newIndex: number }) {
  void reorderRows(payload.oldIndex, payload.newIndex);
}

watch(keyword, () => {
  page.value = 1;
  gridRef.value?.clearInteraction();
});

function onSelectionChange(rows: number[]) {
  selectedRowIds.value = rows;
}

function cancelSelection() {
  gridRef.value?.clearRowSelection();
  selectedRowIds.value = [];
}

async function deleteRows(ids: number[]) {
  if (ids.length === 0) return;
  try {
    await queueWrite(() =>
      api.deleteSheetRows(templateId.value, activeDataSheet.value, ids),
    );
    cancelSelection();
    await reload(false);
    clearHistory();
    notify(`已删除 ${ids.length} 行`);
  } catch (error) {
    notify(String(error), "error");
  }
}

async function clearRows(ids: number[]) {
  const rows = (table.value?.rows ?? []).filter((row) =>
    ids.includes(row.row),
  );
  if (rows.length === 0) return;
  const changes: { row: number; col: number; before: string; after: string }[] = [];
  for (const row of rows) {
    row.cells.forEach((cell, col) => {
      if (cell !== "") changes.push({ row: row.row, col, before: cell, after: "" });
    });
  }
  cancelSelection();
  await writeCellsWithHistory(changes);
}

async function duplicateRows(ids: number[]) {
  const sources = (table.value?.rows ?? []).filter((row) =>
    ids.includes(row.row),
  );
  if (sources.length === 0 || addingRow.value) return;
  addingRow.value = true;
  try {
    for (const row of sources) {
      await api.appendSheetRow(
        templateId.value,
        activeDataSheet.value,
        headers.value.map((_, index) => row.cells[index] ?? ""),
      );
    }
    cancelSelection();
    await reload(false);
    clearHistory();
    notify(`已复制 ${sources.length} 行`);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    addingRow.value = false;
  }
}

async function applyCellChange(payload: {
  row: number;
  col: number;
  value: string;
}) {
  const target = table.value?.rows.find((item) => item.row === payload.row);
  if (!target) return;
  const previous = target.cells[payload.col] ?? "";
  if (payload.value === previous) return;
  const key = dataKeys.value[payload.col];
  const affectsView =
    (key !== undefined && dataSorts.value.some((sort) => sort.key === key)) ||
    (key !== undefined &&
      dataFilters.value.some((filter) => filter.key === key));
  await writeCellsWithHistory([
    { row: payload.row, col: payload.col, before: previous, after: payload.value },
  ]);
  if (affectsView) await reload(false);
}

async function reload(resetPage = true) {
  if (!templateId.value) {
    table.value = null;
    return;
  }
  loading.value = true;
  try {
    // 刚失焦的单元格可能还在写入，先等它落盘再读，避免刷新到旧值。
    await writeChain;
    table.value = activeDataSheet.value
      ? await api.readSheetDataNamed(templateId.value, activeDataSheet.value)
      : await api.readSheetData(templateId.value);
    page.value = resetPage ? 1 : Math.min(page.value, totalPages.value);
    gridRef.value?.clearInteraction();
  } catch (error) {
    table.value = null;
    notify(String(error), "error");
  } finally {
    loading.value = false;
  }
}

/** 读取模板工作簿的 sheet 列表，并切到指定/主工作表。 */
async function loadDataSheets(preferred?: string) {
  clearHistory();
  if (!templateId.value) {
    dataSheetList.value = [];
    activeDataSheet.value = "";
    table.value = null;
    return;
  }
  try {
    const list = await api.readTemplateSheets(templateId.value);
    dataSheetList.value = list;
    const template = templates.value.find((item) => item.id === templateId.value);
    const primary =
      template?.sheets.find((sheet) => sheet.primary)?.name ??
      template?.sheetName ??
      "";
    const target =
      (preferred && list.some((sheet) => sheet.name === preferred)
        ? preferred
        : "") ||
      list.find((sheet) => sheet.name === activeDataSheet.value)?.name ||
      list.find((sheet) => sheet.name === primary)?.name ||
      list[0]?.name ||
      "";
    activeDataSheet.value = target;
    viewConfig.value = readStoredConfig(
      target ? `jy-table-view:${templateId.value}:${target}` : "",
    );
  } catch (error) {
    dataSheetList.value = [];
    notify(String(error), "error");
  }
  await reload(true);
}

function selectDataSheet(name: string) {
  if (!name || name === activeDataSheet.value) return;
  activeDataSheet.value = name;
  viewConfig.value = readStoredConfig(
    `jy-table-view:${templateId.value}:${name}`,
  );
  keyword.value = "";
  page.value = 1;
  gridRef.value?.clearInteraction();
  clearHistory();
  void reload(true);
}

async function addDataSheet() {
  if (!templateId.value || addingDataSheet.value) return;
  addingDataSheet.value = true;
  try {
    const name = await api.addTemplateSheet(templateId.value);
    await loadDataSheets(name);
    notify(`已新建工作表「${name}」`);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    addingDataSheet.value = false;
  }
}

async function renameDataSheet(oldName: string, newName: string) {
  try {
    const wasActive = activeDataSheet.value === oldName;
    await api.renameTemplateSheet(templateId.value, oldName, newName);
    await loadDataSheets(wasActive ? newName : activeDataSheet.value);
    notify(`已重命名为「${newName}」`);
  } catch (error) {
    notify(String(error), "error");
  }
}

function openEdit(row: number, cells: string[]) {
  editingRow.value = row;
  originValues.value = headers.value.map((_, index) => cells[index] ?? "");
  draftValues.value = [...originValues.value];
  deleteConfirmRow.value = null;
}

async function addEmptyRow() {
  if (loading.value || addingRow.value || headers.value.length === 0) return;
  addingRow.value = true;
  try {
    await queueWrite(() =>
      api.appendSheetRow(
        templateId.value,
        activeDataSheet.value,
        headers.value.map(() => ""),
      ),
    );
    keyword.value = "";
    await reload(true);
    page.value = totalPages.value;
    notify("已新增一行");
  } catch (error) {
    notify(String(error), "error");
  } finally {
    addingRow.value = false;
  }
}

function closeDrawer() {
  editingRow.value = null;
  deleteConfirmRow.value = null;
  window.clearTimeout(deleteTimer);
}

async function saveRow() {
  if (editingRow.value === null) return;
  const rowNumber = editingRow.value;
  const values = draftValues.value.map((value) => value.trim());

  const changes: (string | null)[] = values.map((value, index) =>
    value === originValues.value[index] ? null : value,
  );
  if (!changes.some((value) => value !== null)) {
    closeDrawer();
    return;
  }

  saving.value = true;
  try {
    await queueWrite(() =>
      api.updateSheetRow(
        templateId.value,
        activeDataSheet.value,
        rowNumber,
        changes,
      ),
    );
    notify("已保存到表格");
    closeDrawer();
    await reload(false);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    saving.value = false;
  }
}

function requestDelete() {
  if (editingRow.value === null) return;
  if (deleteConfirmRow.value === editingRow.value) {
    void performDelete();
    return;
  }
  deleteConfirmRow.value = editingRow.value;
  window.clearTimeout(deleteTimer);
  deleteTimer = window.setTimeout(() => {
    if (deleteConfirmRow.value === editingRow.value) deleteConfirmRow.value = null;
  }, 3000);
}

async function performDelete() {
  if (editingRow.value === null) return;
  const rowNumber = editingRow.value;
  saving.value = true;
  try {
    await queueWrite(() =>
      api.deleteSheetRow(templateId.value, activeDataSheet.value, rowNumber),
    );
    notify("已删除这一行");
    closeDrawer();
    await reload(false);
    clearHistory();
  } catch (error) {
    notify(String(error), "error");
  } finally {
    saving.value = false;
  }
}

function prevPage() {
  if (page.value > 1) {
    page.value -= 1;
    gridRef.value?.clearInteraction();
  }
}

function nextPage() {
  if (page.value < totalPages.value) {
    page.value += 1;
    gridRef.value?.clearInteraction();
  }
}

function onTemplateChange(id: string) {
  templateId.value = id;
  activeDataSheet.value = "";
  keyword.value = "";
  clearHistory();
  void loadDataSheets();
}

onMounted(async () => {
  window.addEventListener("keydown", onGlobalKeydown);
  try {
    templates.value = await api.listTemplates();
    const preferred =
      templates.value.find((item) => item.isDefault) ?? templates.value[0];
    templateId.value = preferred?.id ?? "";
  } catch (error) {
    notify(String(error), "error");
  }
  await loadDataSheets();
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onGlobalKeydown);
  window.clearTimeout(deleteTimer);
  columnSortable?.destroy();
  columnSortable = null;
});
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div class="ledger-title">
        <div v-if="whitelistOpen" class="whitelist-title-row">
          <button class="btn small" @click="closeWhitelist">← 返回数据</button>
          <div class="page-title">标准名录</div>
        </div>
        <div v-else class="page-title">数据管理</div>
        <div v-if="whitelistOpen" class="page-sub">
          按客户工作簿的工作表分类 · {{ dataSheetList.length }} 个 sheet
          <template v-if="activeDataSheet">
            · 当前 {{ activeDataSheet }} · {{ filteredRows.length }} 行
          </template>
        </div>
        <div v-else class="page-sub ledger-path" :title="table?.path">{{ subtitle }}</div>
      </div>
      <div class="head-actions">
        <DropdownSelect
          :model-value="templateId"
          :options="templateOptions"
          compact
          @change="onTemplateChange"
        />
        <button class="btn" :disabled="loading" @click="reload(false)">
          {{ loading ? "刷新中…" : "刷新" }}
        </button>
        <button
          v-if="!whitelistOpen"
          class="btn"
          :disabled="loading || headers.length === 0"
          @click="openWhitelist"
        >
          标准名录
        </button>
      </div>
    </div>

    <div class="panel ledger-panel">
      <SheetTabs
        class="in-panel"
        :sheets="dataSheetList"
        :active="activeDataSheet"
        :adding="addingDataSheet"
        @select="selectDataSheet"
        @add="addDataSheet"
        @rename="renameDataSheet"
      />
      <div class="table-view-bar">
        <ViewRulesBar
          :headers="headers"
          :keys="dataKeys"
          :formats="dataFormats"
          :config="viewConfig"
          @change="applyViewConfig"
        />
        <div class="view-search">
          <input
            v-model="keyword"
            class="search-input"
            :placeholder="whitelistOpen ? '搜索当前工作表' : '搜索表格内容'"
          />
        </div>
      </div>

      <SheetGrid
        ref="gridRef"
        :headers="headers"
        :rows="visibleRows"
        :sizing-rows="table?.rows ?? []"
        :keys="dataKeys"
        :formats="dataFormats"
        :options="dataOptions"
        :hidden-columns="dataHiddenColumns"
        :group-by="dataGroupColumns"
        :group-directions="dataGroupDirections"
        :freeze-first-column="Boolean(viewConfig.freezeFirstColumn)"
        selectable-rows
        :loading="loading"
        column-editable
        :drag-disabled="dataReorderDisabled"
        :width-cache-key="widthCacheKey"
        :empty-text="table?.rows.length ? '没有匹配的内容' : '表格里还没有数据'"
        @cell-change="applyCellChange"
        @rows-reorder="onRowsReorder"
        @column-rename="renameDataColumn"
        @column-add="addDataColumn"
        @column-action="onDataColumnAction"
        @range-paste="onRangePaste"
        @range-clear="onRangeClear"
        @range-fill="onRangeFill"
        @selection-change="onSelectionChange"
        @cell-contextmenu="openCellMenu"
      >
        <template #row-actions="{ row }">
          <button class="text-btn" @click="openEdit(row.row, row.cells)">
            详情
          </button>
        </template>
      </SheetGrid>

      <div class="ledger-footer">
        <button
          class="ledger-add-row"
          :disabled="loading || addingRow || headers.length === 0"
          @click="addEmptyRow"
        >
          + 新增行
        </button>
        <div
          v-if="filteredRows.length > PAGE_SIZE && dataGroupColumns.length === 0"
          class="ledger-pager"
        >
          <span class="pager-info">第 {{ page }} / {{ totalPages }} 页</span>
          <div class="pager">
            <button class="btn small" :disabled="page <= 1" @click="prevPage">
              上一页
            </button>
            <button class="btn small" :disabled="page >= totalPages" @click="nextPage">
              下一页
            </button>
          </div>
        </div>
      </div>
    </div>

    <Teleport to="body">
      <div
        v-if="cellMenu"
        class="col-menu-backdrop"
        @click="closeCellMenu"
        @contextmenu.prevent="closeCellMenu"
      ></div>
      <div
        v-if="cellMenu"
        class="col-menu context"
        :style="cellMenuStyle"
        @click.stop
        @contextmenu.prevent
      >
        <div class="col-menu-title">单元格操作</div>
        <button class="col-menu-item" @click="copyMenuCell">复制内容</button>
        <button class="col-menu-item" @click="duplicateMenuRows">复制行</button>
        <button class="col-menu-item" @click="clearMenuRows">清空内容</button>
        <button class="col-menu-item danger" @click="deleteMenuRows">删除行</button>
        <div class="col-menu-sep"></div>
        <button class="col-menu-item" @click="replaceFromMenu">替换…</button>
        <button
          class="col-menu-item"
          :disabled="loading || headers.length === 0"
          @click="columnSettingsFromMenu"
        >
          列设置…
        </button>
        <div class="col-menu-sep"></div>
        <button class="col-menu-item" @click="freezeFromMenu">
          冻结首列{{ viewConfig.freezeFirstColumn ? " ✓" : "" }}
        </button>
        <button
          class="col-menu-item"
          :disabled="dataHiddenColumns.length === 0"
          @click="showAllColumnsFromMenu"
        >
          显示全部列
          <template v-if="dataHiddenColumns.length">
            ({{ dataHiddenColumns.length }})
          </template>
        </button>
        <button class="col-menu-item" @click="resetWidthsFromMenu">重置列宽</button>
        <button class="col-menu-item" @click="clearViewRulesFromMenu">
          清空视图规则
        </button>
      </div>
    </Teleport>

    <div v-if="editingRow !== null" class="drawer-backdrop" @click.self="closeDrawer">
      <aside class="drawer">
        <div class="drawer-head">
          <div>
            <div class="modal-title">{{ drawerTitle }}</div>
            <div class="page-sub">{{ table?.sheet }}</div>
          </div>
          <button class="text-btn" @click="closeDrawer">关闭</button>
        </div>
        <div class="drawer-body">
          <div v-for="(header, index) in headers" :key="index" class="field">
            <div class="field-head">
              <span class="field-label">{{ header }}</span>
            </div>
            <select
              v-if="dataFormats[index] === 'select'"
              v-model="draftValues[index]"
              class="field-input"
            >
              <option value="">（空）</option>
              <option v-for="option in dataOptions[index]" :key="option" :value="option">
                {{ option }}
              </option>
            </select>
            <input
              v-else-if="dataFormats[index] === 'date'"
              v-model="draftValues[index]"
              type="date"
              class="field-input"
            />
            <input
              v-else
              v-model="draftValues[index]"
              class="field-input"
              :placeholder="
                dataFormats[index] === 'multi_select'
                  ? '多个选项用逗号分隔'
                  : ''
              "
            />
            <div
              v-if="dataFormats[index] === 'multi_select' && dataOptions[index].length"
              class="hint"
            >
              可选：{{ dataOptions[index].join("、") }}
            </div>
          </div>
          <div v-if="headers.length === 0" class="hint">
            这张表还没有列，请先在设置里配置导出列
          </div>
        </div>
        <div class="drawer-foot">
          <button class="text-btn" @click="requestDelete">
            {{ deleteConfirmRow === editingRow ? "确认删除" : "删除" }}
          </button>
          <div class="drawer-spacer"></div>
          <button class="btn" @click="closeDrawer">取消</button>
          <button
            class="btn primary"
            :disabled="saving || headers.length === 0"
            @click="saveRow"
          >
            {{ saving ? "保存中…" : "保存" }}
          </button>
        </div>
      </aside>
    </div>

    <div v-if="replaceOpen" class="modal-backdrop">
      <div class="modal">
        <div class="modal-head">
          <div>
            <div class="modal-title">查找替换</div>
            <div class="page-sub">按原文精确包含匹配，替换当前工作表</div>
          </div>
          <button class="text-btn" @click="replaceOpen = false">关闭</button>
        </div>
        <div class="modal-body">
          <div class="field">
            <div class="field-head">
              <span class="field-label">查找内容</span>
            </div>
            <input v-model="replaceFind" class="field-input" />
          </div>
          <div class="field">
            <div class="field-head">
              <span class="field-label">替换为</span>
            </div>
            <input v-model="replaceWith" class="field-input" />
          </div>
        </div>
        <div class="modal-foot">
          <button class="btn" :disabled="replaceBusy" @click="replaceOpen = false">
            取消
          </button>
          <button
            class="btn primary"
            :disabled="replaceBusy || !replaceFind"
            @click="runReplace"
          >
            {{ replaceBusy ? "替换中…" : "全部替换" }}
          </button>
        </div>
      </div>
    </div>

    <div v-if="columnEditorOpen" class="modal-backdrop">
      <div class="modal wide">
        <div class="modal-head">
          <div>
            <div class="modal-title">列设置</div>
            <div class="page-sub">
              {{ table?.sheet }} · {{ columnDrafts.length }} 列
            </div>
          </div>
          <button class="text-btn" @click="closeColumnEditor">关闭</button>
        </div>
        <div class="modal-body column-settings-body">
          <div class="template-col-row head">
            <span></span>
            <span>列名</span>
            <span>示例</span>
            <span>格式</span>
            <span></span>
          </div>
          <div ref="columnListRef" class="template-col-list">
            <div
              v-for="draft in columnDrafts"
              :key="draft.uid"
              class="template-col-row"
            >
              <span class="drag-handle">⠿</span>
              <input v-model="draft.label" class="field-input" placeholder="列名" />
              <input v-model="draft.hint" class="field-input" placeholder="示例值" />
              <select v-model="draft.format" class="field-input">
                <option value="text">文本</option>
                <option value="number">数字</option>
                <option value="date">日期</option>
                <option value="select">单选</option>
                <option value="multi_select">多选</option>
              </select>
              <div class="row-actions">
                <button class="text-btn" @click="removeColumnDraft(draft.uid)">
                  删除
                </button>
              </div>
            </div>
          </div>
          <div class="row-actions template-actions">
            <button class="btn small" @click="addColumnDraft">新增列</button>
          </div>
        </div>
        <div class="modal-foot">
          <button class="btn" :disabled="columnBusy" @click="closeColumnEditor">
            取消
          </button>
          <button
            class="btn primary"
            :disabled="columnBusy"
            @click="saveColumnSettings"
          >
            {{ columnBusy ? "保存中…" : "保存" }}
          </button>
        </div>
      </div>
    </div>
  </div>

  <div v-if="filterEditor" class="modal-backdrop">
    <div class="modal">
      <div class="modal-head">
        <div>
          <div class="modal-title">
            筛选 · {{ filterColumnLabel() }}
          </div>
          <div class="page-sub">只影响当前视图，不会删除数据</div>
        </div>
        <button class="text-btn" @click="filterEditor = null">关闭</button>
      </div>
      <div class="modal-body">
        <div class="field">
          <div class="field-head">
            <span class="field-label">条件</span>
          </div>
          <DropdownSelect
            :model-value="filterEditor.operator"
            :options="filterOperatorOptions"
            @change="onFilterOperatorChange"
          />
        </div>
        <div
          v-if="filterEditor.operator !== 'empty' && filterEditor.operator !== 'notEmpty'"
          class="field"
        >
          <div class="field-head">
            <span class="field-label">值</span>
          </div>
          <input
            v-model="filterEditor.value"
            class="field-input"
            placeholder="输入筛选值"
            @keydown.enter="saveFilter"
          />
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn" @click="filterEditor = null">取消</button>
        <button class="btn primary" @click="saveFilter">应用筛选</button>
      </div>
    </div>
  </div>

  <div v-if="columnSettings" class="modal-backdrop">
    <div class="modal">
      <div class="modal-head">
        <div>
          <div class="modal-title">编辑列</div>
          <div class="page-sub">列类型与选项只保存在本地，不写入 Excel</div>
        </div>
        <button class="text-btn" @click="columnSettings = null">关闭</button>
      </div>
      <div class="modal-body">
        <div class="field">
          <div class="field-head">
            <span class="field-label">列名</span>
          </div>
          <input v-model="columnSettings.label" class="field-input" />
        </div>
        <div class="field">
          <div class="field-head">
            <span class="field-label">类型</span>
          </div>
          <DropdownSelect
            :model-value="columnSettings.format"
            :options="columnFormatOptions"
            @change="onColumnFormatChange"
          />
        </div>
        <div
          v-if="columnSettings.format === 'select' || columnSettings.format === 'multi_select'"
          class="field"
        >
          <div class="field-head">
            <span class="field-label">选项</span>
          </div>
          <input
            v-model="columnSettings.options"
            class="field-input"
            placeholder="用逗号分隔，例如：已联系，待跟进，已完成"
          />
        </div>
        <div class="field">
          <div class="field-head">
            <span class="field-label">填写示例</span>
          </div>
          <input v-model="columnSettings.hint" class="field-input" />
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn" @click="columnSettings = null">取消</button>
        <button class="btn primary" @click="saveColumnSetting">保存</button>
      </div>
    </div>
  </div>
</template>

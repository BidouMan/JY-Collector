<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import Sortable from "sortablejs";
import { api } from "../api";
import DropdownSelect from "../components/DropdownSelect.vue";
import SheetGrid from "../components/SheetGrid.vue";
import { notify } from "../toast";
import type { SheetInfo, SheetTable, Template } from "../types";

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

interface SheetGridApi {
  clearInteraction: () => void;
  resetWidths: () => void;
}
const gridRef = ref<SheetGridApi | null>(null);
const widthCacheKey = computed(() =>
  templateId.value ? `jy-ledger-widths:${templateId.value}` : "",
);

interface ColumnDraft {
  uid: number;
  source: number;
  label: string;
  hint: string;
  width: number;
  format: "text" | "number" | "date";
}

const columnEditorOpen = ref(false);
const columnDrafts = ref<ColumnDraft[]>([]);
const columnBusy = ref(false);
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

// 标准名录：直接按客户 Excel 的工作表浏览，每个 sheet 就是一个分类。
const whitelistOpen = ref(false);
const whitelistKeyword = ref("");
const whitelistLoading = ref(false);
const whitelistPage = ref(1);
const sheetList = ref<SheetInfo[]>([]);
const activeSheet = ref("");
const sheetTable = ref<SheetTable | null>(null);

const whitelistHeaders = computed(() => sheetTable.value?.headers ?? []);

const whitelistFilteredRows = computed(() => {
  const rows = sheetTable.value?.rows ?? [];
  const query = whitelistKeyword.value.trim().toLowerCase();
  if (!query) return rows;
  return rows.filter((row) =>
    row.cells.some((cell) => cell.toLowerCase().includes(query)),
  );
});

const whitelistTotalPages = computed(() =>
  Math.max(1, Math.ceil(whitelistFilteredRows.value.length / PAGE_SIZE)),
);

const whitelistRows = computed(() => {
  const start = (whitelistPage.value - 1) * PAGE_SIZE;
  return whitelistFilteredRows.value.slice(start, start + PAGE_SIZE);
});

const whitelistWidthKey = computed(() =>
  activeSheet.value ? `jy-whitelist-widths:${activeSheet.value}` : "",
);

async function loadSheet(name: string) {
  if (!name || !templateId.value) {
    sheetTable.value = null;
    return;
  }
  whitelistLoading.value = true;
  try {
    sheetTable.value = await api.readSheetDataNamed(templateId.value, name);
    activeSheet.value = name;
    whitelistPage.value = 1;
  } catch (error) {
    sheetTable.value = null;
    notify(String(error), "error");
  } finally {
    whitelistLoading.value = false;
  }
}

async function openWhitelist() {
  whitelistKeyword.value = "";
  whitelistPage.value = 1;
  sheetList.value = [];
  sheetTable.value = null;
  activeSheet.value = "";
  whitelistOpen.value = true;
  if (!templateId.value) return;
  whitelistLoading.value = true;
  try {
    sheetList.value = await api.readTemplateSheets(templateId.value);
    const first = sheetList.value[0]?.name ?? "";
    if (first) await loadSheet(first);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    whitelistLoading.value = false;
  }
}

function selectSheet(name: string) {
  if (name === activeSheet.value) return;
  whitelistKeyword.value = "";
  whitelistPage.value = 1;
  void loadSheet(name);
}

watch(whitelistKeyword, () => {
  whitelistPage.value = 1;
});

function prevWhitelistPage() {
  if (whitelistPage.value > 1) whitelistPage.value -= 1;
}

function nextWhitelistPage() {
  if (whitelistPage.value < whitelistTotalPages.value) whitelistPage.value += 1;
}

const drawerTitle = computed(() => `编辑第 ${editingRow.value} 行`);
const subtitle = computed(() => {
  if (!table.value) return loading.value ? "读取中…" : "表格尚未生成";
  return `${table.value.sheet} · ${table.value.rows.length} 行 · ${table.value.path}`;
});

const filteredRows = computed(() => {
  const rows = table.value?.rows ?? [];
  const query = keyword.value.trim().toLowerCase();
  if (!query) return rows;
  return rows.filter((row) =>
    row.cells.some((cell) => cell.toLowerCase().includes(query)),
  );
});

const totalPages = computed(() =>
  Math.max(1, Math.ceil(filteredRows.value.length / PAGE_SIZE)),
);

const visibleRows = computed(() => {
  const start = (page.value - 1) * PAGE_SIZE;
  return filteredRows.value.slice(start, start + PAGE_SIZE);
});

function openColumnEditor() {
  if (headers.value.length === 0) {
    notify("请先在设置里配置导出列", "error");
    return;
  }
  const template = templates.value.find((item) => item.id === templateId.value);
  columnDrafts.value = headers.value.map((label, index) => {
    const meta = template?.columns[index];
    return {
      uid: ++columnSeq,
      source: index + 1,
      label,
      hint: meta?.hint ?? "",
      width: meta?.width ?? 18,
      format: meta?.format ?? "text",
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
  if (new Set(labels).size !== labels.length) {
    notify("列名不能重复", "error");
    return;
  }
  const payload = drafts.map((draft) => ({
    source: draft.source,
    label: draft.label.trim(),
    hint: draft.hint.trim(),
    width: Math.min(80, Math.max(6, Number(draft.width) || 18)),
    format: draft.format,
  }));
  columnBusy.value = true;
  try {
    await queueWrite(() => api.saveSheetColumns(templateId.value, payload));
    gridRef.value?.resetWidths();
    await reload(false);
    closeColumnEditor();
    notify("列设置已保存");
  } catch (error) {
    notify(String(error), "error");
  } finally {
    columnBusy.value = false;
  }
}

async function reorderRows(oldIndex: number, newIndex: number) {  if (keyword.value.trim()) return;
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
    await queueWrite(() => api.reorderSheetRows(templateId.value, order));
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

async function applyCellChange(payload: {
  row: number;
  col: number;
  value: string;
}) {
  const target = table.value?.rows.find((item) => item.row === payload.row);
  if (!target) return;
  const previous = target.cells[payload.col] ?? "";
  if (payload.value === previous) return;
  target.cells[payload.col] = payload.value;
  try {
    const changes: (string | null)[] = headers.value.map((_, index) =>
      index === payload.col ? payload.value : null,
    );
    await queueWrite(() =>
      api.updateSheetRow(templateId.value, payload.row, changes),
    );
  } catch (error) {
    if (target.cells[payload.col] === payload.value) {
      target.cells[payload.col] = previous;
    }
    notify(String(error), "error");
  }
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
    table.value = await api.readSheetData(templateId.value);
    page.value = resetPage ? 1 : Math.min(page.value, totalPages.value);
    gridRef.value?.clearInteraction();
  } catch (error) {
    table.value = null;
    notify(String(error), "error");
  } finally {
    loading.value = false;
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
      api.updateSheetRow(templateId.value, rowNumber, changes),
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
    await queueWrite(() => api.deleteSheetRow(templateId.value, rowNumber));
    notify("已删除这一行");
    closeDrawer();
    await reload(false);
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
  void reload(true);
}

onMounted(async () => {
  try {
    templates.value = await api.listTemplates();
    const preferred =
      templates.value.find((item) => item.isDefault) ?? templates.value[0];
    templateId.value = preferred?.id ?? "";
  } catch (error) {
    notify(String(error), "error");
  }
  await reload();
});

onBeforeUnmount(() => {
  window.clearTimeout(deleteTimer);
  columnSortable?.destroy();
  columnSortable = null;
});
</script>

<template>
  <div v-if="whitelistOpen" class="page">
    <div class="page-head">
      <div>
        <div class="whitelist-title-row">
          <button class="btn small" @click="whitelistOpen = false">← 返回数据</button>
          <div class="page-title">标准名录</div>
        </div>
        <div class="page-sub">
          按客户工作簿的工作表分类 · {{ sheetList.length }} 个 sheet
          <template v-if="activeSheet">
            · 当前 {{ activeSheet }} · {{ whitelistFilteredRows.length }} 行
          </template>
        </div>
      </div>
    </div>

    <div v-if="sheetList.length > 0" class="sheet-tabs">
      <button
        v-for="sheet in sheetList"
        :key="sheet.name"
        class="sheet-tab"
        :class="{ active: sheet.name === activeSheet }"
        @click="selectSheet(sheet.name)"
      >
        <span class="sheet-tab-name">{{ sheet.name }}</span>
        <span class="sheet-tab-count">{{ sheet.rows }}</span>
      </button>
    </div>

    <div class="panel">
      <div class="filter-bar">
        <div class="grow">
          <input
            v-model="whitelistKeyword"
            class="search-input"
            placeholder="搜索当前工作表"
          />
        </div>
        <span class="ledger-count">{{ whitelistFilteredRows.length }} 行</span>
      </div>

      <SheetGrid
        :headers="whitelistHeaders"
        :rows="whitelistRows"
        :loading="whitelistLoading"
        :editable="false"
        :draggable="false"
        :resizable="true"
        :width-cache-key="whitelistWidthKey"
        :min-width="0"
        :empty-text="
          !activeSheet
            ? '当前配置还没有可浏览的工作表'
            : whitelistKeyword.trim()
              ? '没有匹配的内容'
              : '这个工作表里还没有数据'
        "
      />

      <div
        v-if="whitelistFilteredRows.length > PAGE_SIZE"
        class="ledger-footer"
      >
        <span class="pager-info">
          第 {{ whitelistPage }} / {{ whitelistTotalPages }} 页
        </span>
        <div class="pager">
          <button
            class="btn small"
            :disabled="whitelistPage <= 1"
            @click="prevWhitelistPage"
          >
            上一页
          </button>
          <button
            class="btn small"
            :disabled="whitelistPage >= whitelistTotalPages"
            @click="nextWhitelistPage"
          >
            下一页
          </button>
        </div>
      </div>
    </div>
  </div>

  <div v-else class="page">
    <div class="page-head">
      <div class="ledger-title">
        <div class="page-title">数据管理</div>
        <div class="page-sub ledger-path" :title="table?.path">{{ subtitle }}</div>
      </div>
      <div class="head-actions">
        <DropdownSelect
          :model-value="templateId"
          :options="templateOptions"
          compact
          @change="onTemplateChange"
        />
        <button
          class="btn"
          :disabled="loading || headers.length === 0"
          @click="openColumnEditor"
        >
          列设置
        </button>
        <button class="btn" :disabled="loading" @click="reload(false)">
          {{ loading ? "刷新中…" : "刷新" }}
        </button>
        <button
          class="btn"
          :disabled="loading || headers.length === 0"
          @click="openWhitelist"
        >
          标准名录
        </button>
      </div>
    </div>

    <div class="panel ledger-panel">
      <div class="filter-bar">
        <div class="grow">
          <input v-model="keyword" class="search-input" placeholder="搜索表格内容" />
        </div>
        <span class="ledger-count">{{ filteredRows.length }} 行</span>
      </div>

      <SheetGrid
        ref="gridRef"
        :headers="headers"
        :rows="visibleRows"
        :sizing-rows="table?.rows ?? []"
        :loading="loading"
        :drag-disabled="keyword.trim() !== ''"
        :width-cache-key="widthCacheKey"
        :empty-text="table?.rows.length ? '没有匹配的内容' : '表格里还没有数据'"
        @cell-change="applyCellChange"
        @rows-reorder="onRowsReorder"
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
        <div v-if="filteredRows.length > PAGE_SIZE" class="ledger-pager">
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
            <input v-model="draftValues[index]" class="field-input" />
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
</template>

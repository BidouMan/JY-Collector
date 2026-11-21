<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { api } from "../api";
import { notify } from "../toast";
import type { SheetTable, Template } from "../types";

const PAGE_SIZE = 100;

const templates = ref<Template[]>([]);
const templateId = ref("");
const table = ref<SheetTable | null>(null);
const loading = ref(false);
const keyword = ref("");
const page = ref(1);

const editingRow = ref<number | null>(null);
const draftValues = ref<string[]>([]);
const originValues = ref<string[]>([]);
const saving = ref(false);
const deleteConfirmRow = ref<number | null>(null);
let deleteTimer: number | undefined;

const headers = computed(() => table.value?.headers ?? []);
const isNew = computed(() => editingRow.value === 0);
const drawerTitle = computed(() =>
  isNew.value ? "新增行" : `编辑第 ${editingRow.value} 行`,
);
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

watch(keyword, () => {
  page.value = 1;
});

async function reload(resetPage = true) {
  if (!templateId.value) {
    table.value = null;
    return;
  }
  loading.value = true;
  try {
    table.value = await api.readSheetData(templateId.value);
    page.value = resetPage ? 1 : Math.min(page.value, totalPages.value);
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

function openCreate() {
  if (headers.value.length === 0) {
    notify("请先在设置里配置导出列", "error");
    return;
  }
  editingRow.value = 0;
  originValues.value = headers.value.map(() => "");
  draftValues.value = headers.value.map(() => "");
  deleteConfirmRow.value = null;
}

function closeDrawer() {
  editingRow.value = null;
  deleteConfirmRow.value = null;
  window.clearTimeout(deleteTimer);
}

async function saveRow() {
  if (editingRow.value === null) return;
  const values = draftValues.value.map((value) => value.trim());

  if (isNew.value) {
    if (!values.some((value) => value !== "")) {
      notify("请至少填写一列", "error");
      return;
    }
    saving.value = true;
    try {
      await api.appendSheetRow(templateId.value, values);
      notify("已新增一行");
      keyword.value = "";
      closeDrawer();
      await reload(true);
      page.value = totalPages.value;
    } catch (error) {
      notify(String(error), "error");
    } finally {
      saving.value = false;
    }
    return;
  }

  const changes: (string | null)[] = values.map((value, index) =>
    value === originValues.value[index] ? null : value,
  );
  if (!changes.some((value) => value !== null)) {
    closeDrawer();
    return;
  }

  saving.value = true;
  try {
    await api.updateSheetRow(templateId.value, editingRow.value, changes);
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
  if (editingRow.value === null || isNew.value) return;
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
  saving.value = true;
  try {
    await api.deleteSheetRow(templateId.value, editingRow.value);
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
  if (page.value > 1) page.value -= 1;
}

function nextPage() {
  if (page.value < totalPages.value) page.value += 1;
}

function onTemplateChange(event: Event) {
  templateId.value = (event.target as HTMLSelectElement).value;
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
});
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div class="ledger-title">
        <div class="page-title">台账</div>
        <div class="page-sub ledger-path" :title="table?.path">{{ subtitle }}</div>
      </div>
      <div class="head-actions">
        <select
          :value="templateId"
          class="field-input compact"
          @change="onTemplateChange"
        >
          <option v-for="item in templates" :key="item.id" :value="item.id">
            {{ item.name }}
          </option>
        </select>
        <button class="btn" :disabled="loading" @click="reload(false)">
          {{ loading ? "刷新中…" : "刷新" }}
        </button>
        <button
          class="btn primary"
          :disabled="loading || headers.length === 0"
          @click="openCreate"
        >
          新增行
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

      <div class="table-wrap ledger-table-wrap">
        <table class="records ledger-table">
          <thead>
            <tr>
              <th class="row-no">行</th>
              <th v-for="(header, index) in headers" :key="index">{{ header }}</th>
              <th class="actions-col"></th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="row in visibleRows"
              :key="row.row"
              @click="openEdit(row.row, row.cells)"
            >
              <td class="row-no">{{ row.row }}</td>
              <td v-for="(cell, index) in row.cells" :key="index" :title="cell">
                <span class="cell-text">{{ cell }}</span>
              </td>
              <td class="actions-col">
                <button class="text-btn" @click.stop="openEdit(row.row, row.cells)">
                  编辑
                </button>
              </td>
            </tr>
          </tbody>
        </table>
        <div v-if="loading" class="table-empty">加载中</div>
        <div v-else-if="filteredRows.length === 0" class="table-empty">
          {{ table?.rows.length ? "没有匹配的内容" : "表格里还没有数据" }}
        </div>
      </div>

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
          <button v-if="!isNew" class="text-btn" @click="requestDelete">
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
  </div>
</template>

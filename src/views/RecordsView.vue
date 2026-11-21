<script setup lang="ts">
import { save } from "@tauri-apps/plugin-dialog";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api } from "../api";
import MessageDetailModal from "../components/MessageDetailModal.vue";
import {
  appendSummary,
  ensureTargetPath,
  fileNameFor,
} from "../services/ledger";
import { notify } from "../toast";
import type {
  FieldSchema,
  Group,
  Message,
  RecordItem,
  RecordSource,
  Template,
} from "../types";
import { formatTime } from "../utils";

const records = ref<RecordItem[]>([]);
const groups = ref<Group[]>([]);
const schemas = ref<FieldSchema[]>([]);
const templates = ref<Template[]>([]);
const templateId = ref("");
const filterGroupId = ref("");
const keyword = ref("");
const loading = ref(false);
const exporting = ref(false);
const appending = ref(false);

const template = computed(
  () => templates.value.find((item) => item.id === templateId.value) ?? templates.value[0],
);

const editRecord = ref<RecordItem | null>(null);
const editTitle = ref("");
const editData = ref<Record<string, string>>({});
const editSources = ref<RecordSource[]>([]);
const editSaving = ref(false);

const sourceRecord = ref<RecordItem | null>(null);
const sources = ref<RecordSource[]>([]);
const sourcesLoading = ref(false);
const messageModal = ref<{ message: Message; snippet?: string | null } | null>(null);

const deleteConfirmId = ref<string | null>(null);
let deleteTimer: number | undefined;

const filteredRecords = computed(() => {
  const query = keyword.value.trim().toLowerCase();
  return records.value.filter((record) => {
    if (filterGroupId.value && record.groupId !== filterGroupId.value) return false;
    if (!query) return true;
    return (
      record.title.toLowerCase().includes(query) ||
      record.groupName.toLowerCase().includes(query) ||
      Object.values(record.data).some((value) => value.toLowerCase().includes(query))
    );
  });
});

const editSchema = computed(() => {
  if (!editRecord.value) return [];
  const record = editRecord.value;
  const matched =
    templates.value.find((item) => item.id === record.templateId) ?? template.value;
  if (matched) {
    return matched.columns.map((column) => ({
      key: column.key,
      label: column.label,
      fieldType: column.format === "number" ? "number" : "text",
      required: false,
      example: "",
    }));
  }
  return schemaFor(record.groupId);
});

function schemaFor(groupId: string) {
  return schemas.value
    .filter((schema) => schema.groupId === groupId)
    .sort((a, b) => a.sortOrder - b.sortOrder);
}

function fieldSummary(record: RecordItem) {
  const matched =
    templates.value.find((item) => item.id === record.templateId) ?? template.value;
  if (matched) {
    return matched.columns
      .filter((column) => column.key !== "excerpt")
      .map((column) => ({ label: column.label, value: record.data[column.key] ?? "" }))
      .filter((item) => item.value)
      .slice(0, 4);
  }
  const list = schemaFor(record.groupId)
    .map((schema) => ({ label: schema.label, value: record.data[schema.key] ?? "" }))
    .filter((item) => item.value);
  return list.length > 0
    ? list
    : Object.entries(record.data).map(([key, value]) => ({ label: key, value }));
}

function uniqueSources(list: RecordSource[]): RecordSource[] {
  const seen = new Set<string>();
  return list.filter((source) => {
    const key = `${source.message.id}|${source.snippet}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

async function reload() {
  loading.value = true;
  try {
    records.value = await api.listRecords();
  } catch (error) {
    notify(String(error), "error");
  } finally {
    loading.value = false;
  }
}

async function openEdit(record: RecordItem) {
  editRecord.value = record;
  editTitle.value = record.title;
  const data: Record<string, string> = { ...record.data };
  for (const schema of schemaFor(record.groupId)) {
    if (!(schema.key in data)) data[schema.key] = "";
  }
  editData.value = data;
  editSources.value = [];
  try {
    editSources.value = uniqueSources(await api.recordSources(record.id));
  } catch {
    editSources.value = [];
  }
}

function closeEdit() {
  editRecord.value = null;
  editSources.value = [];
}

async function saveEdit() {
  if (!editRecord.value) return;
  const missing = editSchema.value.filter(
    (schema) => schema.required && !editData.value[schema.key]?.trim(),
  );
  if (missing.length > 0) {
    notify(`请填写：${missing.map((schema) => schema.label).join("、")}`, "error");
    return;
  }
  const data: Record<string, string> = {};
  for (const [key, value] of Object.entries(editData.value)) {
    if (value.trim()) data[key] = value.trim();
  }
  editSaving.value = true;
  try {
    await api.updateRecord(editRecord.value.id, editTitle.value.trim(), data);
    notify("记录已更新");
    closeEdit();
    await reload();
  } catch (error) {
    notify(String(error), "error");
  } finally {
    editSaving.value = false;
  }
}

async function openSources(record: RecordItem) {
  sourceRecord.value = record;
  sourcesLoading.value = true;
  sources.value = [];
  try {
    sources.value = uniqueSources(await api.recordSources(record.id));
  } catch (error) {
    notify(String(error), "error");
  } finally {
    sourcesLoading.value = false;
  }
}

function openSourceMessage(source: RecordSource) {
  messageModal.value = { message: source.message, snippet: source.snippet };
}

function requestDelete(record: RecordItem) {
  if (deleteConfirmId.value === record.id) {
    void performDelete(record);
    return;
  }
  deleteConfirmId.value = record.id;
  window.clearTimeout(deleteTimer);
  deleteTimer = window.setTimeout(() => {
    if (deleteConfirmId.value === record.id) deleteConfirmId.value = null;
  }, 3000);
}

async function performDelete(record: RecordItem) {
  deleteConfirmId.value = null;
  try {
    await api.deleteRecord(record.id);
    notify("记录已删除");
    await reload();
  } catch (error) {
    notify(String(error), "error");
  }
}

async function appendLedger() {
  const tpl = template.value;
  if (!tpl) {
    notify("没有可用的导出配置", "error");
    return;
  }
  if (filteredRecords.value.length === 0) {
    notify("当前没有可追加的记录", "error");
    return;
  }
  appending.value = true;
  try {
    const path = await ensureTargetPath(tpl);
    if (!path) return;
    const report = await api.appendLedger(
      filteredRecords.value.map((record) => record.id),
      tpl.id,
      path,
      false,
    );
    notify(appendSummary(report));
    await reload();
  } catch (error) {
    notify(String(error), "error");
  } finally {
    appending.value = false;
  }
}

async function exportAs() {
  const tpl = template.value;
  if (!tpl) {
    notify("没有可用的导出配置", "error");
    return;
  }
  if (filteredRecords.value.length === 0) {
    notify("当前没有可导出的记录", "error");
    return;
  }
  let path: string | null = null;
  const fallbackName = fileNameFor(tpl);
  const dir = tpl.targetPath.toLowerCase().endsWith(".xlsx")
    ? tpl.targetPath.slice(0, tpl.targetPath.lastIndexOf("/"))
    : tpl.targetPath.replace(/\/+$/, "");
  try {
    path = await save({
      title: "另存为",
      defaultPath: dir ? `${dir}/${fallbackName}` : fallbackName,
      filters: [{ name: "Excel 工作簿", extensions: ["xlsx"] }],
    });
  } catch (error) {
    notify(String(error), "error");
    return;
  }
  if (!path) return;
  if (!path.toLowerCase().endsWith(".xlsx")) path += ".xlsx";

  exporting.value = true;
  try {
    const count = await api.exportRecords(
      filteredRecords.value.map((record) => record.id),
      tpl.id,
      path,
    );
    notify(`已导出 ${count} 条记录`);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    exporting.value = false;
  }
}

onMounted(async () => {
  try {
    const [data, templateList] = await Promise.all([api.bootstrap(), api.listTemplates()]);
    groups.value = data.groups;
    schemas.value = data.schemas;
    templates.value = templateList;
    const preferred = templateList.find((item) => item.isDefault) ?? templateList[0];
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
      <div>
        <div class="page-title">记录表</div>
        <div class="page-sub">{{ filteredRecords.length }} / {{ records.length }} 条记录</div>
      </div>
      <div class="head-actions">
        <select v-model="templateId" class="field-input compact">
          <option v-for="item in templates" :key="item.id" :value="item.id">
            {{ item.name }}
          </option>
        </select>
        <button class="btn" :disabled="exporting" @click="exportAs">
          {{ exporting ? "导出中…" : "另存为" }}
        </button>
        <button class="btn primary" :disabled="appending" @click="appendLedger">
          {{ appending ? "追加中…" : "追加导出" }}
        </button>
      </div>
    </div>

    <div class="panel">
      <div class="filter-bar">
        <select v-model="filterGroupId" class="field-input">
          <option value="">全部群组</option>
          <option v-for="group in groups" :key="group.id" :value="group.id">
            {{ group.name }}
          </option>
        </select>
        <div class="grow">
          <input v-model="keyword" class="search-input" placeholder="搜索记录标题或字段内容" />
        </div>
      </div>

      <div class="table-wrap">
        <table class="records">
          <thead>
            <tr>
              <th>记录</th>
              <th>群组</th>
              <th>字段</th>
              <th>来源</th>
              <th>导出</th>
              <th>更新时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="record in filteredRecords" :key="record.id">
              <td>
                <div class="cell-title">{{ record.title }}</div>
                <div class="cell-sub">创建于 {{ formatTime(record.createdAt) }}</div>
              </td>
              <td>{{ record.groupName }}</td>
              <td>
                <div class="cell-fields">
                  <template v-for="(item, index) in fieldSummary(record)" :key="item.label">
                    <span v-if="index > 0" class="sep">·</span>
                    {{ item.label }}：{{ item.value }}
                  </template>
                </div>
              </td>
              <td>
                <button class="text-btn" @click="openSources(record)">
                  {{ record.sourceCount }} 条
                </button>
              </td>
              <td>
                <span class="export-state" :class="{ done: record.exportedAt }">
                  {{ record.exportedAt ? "已导出" : "未导出" }}
                </span>
              </td>
              <td>{{ formatTime(record.updatedAt) }}</td>
              <td>
                <div class="row-actions">
                  <button class="text-btn" @click="openEdit(record)">编辑</button>
                  <button class="text-btn" @click="requestDelete(record)">
                    {{ deleteConfirmId === record.id ? "确认删除" : "删除" }}
                  </button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
        <div v-if="loading" class="table-empty">加载中</div>
        <div v-else-if="filteredRecords.length === 0" class="table-empty">
          没有匹配的记录
        </div>
      </div>
    </div>

    <div v-if="editRecord" class="drawer-backdrop" @click.self="closeEdit">
      <aside class="drawer">
        <div class="drawer-head">
          <div>
            <div class="modal-title">编辑记录</div>
            <div class="page-sub">{{ editRecord.groupName }}</div>
          </div>
          <button class="text-btn" @click="closeEdit">关闭</button>
        </div>
        <div class="drawer-body">
          <div class="field">
            <div class="field-head">
              <span class="field-label">记录标题</span>
            </div>
            <input v-model="editTitle" class="field-input" />
          </div>

          <div v-for="schema in editSchema" :key="schema.key" class="field">
            <div class="field-head">
              <span class="field-label">
                {{ schema.label }}
                <span v-if="schema.required" class="req">*</span>
              </span>
            </div>
            <textarea
              v-if="schema.fieldType === 'textarea'"
              v-model="editData[schema.key]"
              class="field-input"
              :placeholder="schema.example"
            ></textarea>
            <input
              v-else
              v-model="editData[schema.key]"
              class="field-input"
              :type="schema.fieldType === 'number' ? 'number' : schema.fieldType === 'date' ? 'date' : 'text'"
              :placeholder="schema.example"
            />
          </div>

          <div class="drawer-section">
            <div class="drawer-section-title">来源回链</div>
            <div
              v-for="source in editSources"
              :key="`${source.fieldKey}-${source.message.id}`"
              class="source-card"
              @click="openSourceMessage(source)"
            >
              <div class="source-card-meta">
                <span>{{ source.message.senderName }} · {{ source.message.groupName }}</span>
                <span>{{ formatTime(source.message.createdAt) }}</span>
              </div>
              <div class="source-card-text">{{ source.snippet }}</div>
            </div>
            <div v-if="editSources.length === 0" class="hint">没有来源记录</div>
          </div>
        </div>
        <div class="drawer-foot">
          <button class="btn" @click="closeEdit">取消</button>
          <button class="btn primary" :disabled="editSaving" @click="saveEdit">
            {{ editSaving ? "保存中…" : "保存" }}
          </button>
        </div>
      </aside>
    </div>

    <div v-if="sourceRecord" class="modal-backdrop" @click.self="sourceRecord = null">
      <div class="modal">
        <div class="modal-head">
          <div class="modal-title">来源消息</div>
          <button class="text-btn" @click="sourceRecord = null">关闭</button>
        </div>
        <div class="modal-body">
          <div v-if="sourcesLoading" class="extract-loading">加载中…</div>
          <div
            v-for="source in sources"
            :key="`${source.fieldKey}-${source.message.id}`"
            class="source-card"
            @click="openSourceMessage(source)"
          >
            <div class="source-card-meta">
              <span>{{ source.message.senderName }} · {{ source.message.groupName }}</span>
              <span>{{ formatTime(source.message.createdAt) }}</span>
            </div>
            <div class="source-card-text">{{ source.snippet }}</div>
          </div>
          <div v-if="!sourcesLoading && sources.length === 0" class="hint">没有来源记录</div>
        </div>
      </div>
    </div>

    <MessageDetailModal
      v-if="messageModal"
      :message="messageModal.message"
      :snippet="messageModal.snippet"
      @close="messageModal = null"
    />
  </div>
</template>

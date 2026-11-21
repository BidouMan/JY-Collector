<script setup lang="ts">
import { open } from "@tauri-apps/plugin-dialog";
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import Sortable from "sortablejs";
import { api } from "../api";
import { notify } from "../toast";
import type { SheetInfo, Template } from "../types";

const emit = defineEmits<{ changed: [] }>();

const templates = ref<Template[]>([]);
const templateDraft = ref<Template | null>(null);
const savingTemplate = ref(false);
const templateDeleteId = ref<string | null>(null);
const sheets = ref<SheetInfo[]>([]);
const loadingSheets = ref(false);
const columnListRef = ref<HTMLElement | null>(null);
let columnSortable: Sortable | null = null;
let templateDeleteTimer: number | undefined;

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
      const columns = templateDraft.value?.columns;
      if (!columns) return;
      const { oldIndex, newIndex } = event;
      if (oldIndex == null || newIndex == null || oldIndex === newIndex) return;
      const [moved] = columns.splice(oldIndex, 1);
      if (moved) columns.splice(newIndex, 0, moved);
    },
  });
}

watch(columnListRef, initColumnSortable, { flush: "post" });

function workbookFile(template: Template): string {
  const dir = template.targetPath.replace(/\/+$/, "");
  const name = (template.name.trim() || "默认导出").replace(/[\/\\:*?"<>|]/g, "_");
  const fileName = `${name}.xlsx`;
  return dir ? `${dir}/${fileName}` : fileName;
}

function applyHeaders(headers: string[]) {
  const draft = templateDraft.value;
  if (!draft || headers.length === 0) return;
  draft.columns = headers.map((label, index) => {
    const existing = draft.columns.find((column) => column.label === label);
    return {
      key: existing?.key || `col_${index + 1}`,
      label,
      width: existing?.width ?? 18,
      format: existing?.format ?? "text",
      hint: existing?.hint ?? "",
    };
  });
}

async function loadSheets(fillColumns: boolean) {
  const draft = templateDraft.value;
  if (!draft?.targetPath) {
    sheets.value = [];
    return;
  }
  loadingSheets.value = true;
  try {
    const list = await api.readWorkbook(workbookFile(draft));
    sheets.value = list;
    if (!list.length) return;
    const current = list.find((sheet) => sheet.name === draft.sheetName) ?? list[0];
    if (!list.some((sheet) => sheet.name === draft.sheetName)) {
      draft.sheetName = current.name;
    }
    if (fillColumns) applyHeaders(current.headers);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    loadingSheets.value = false;
  }
}

async function onSheetChange() {
  const draft = templateDraft.value;
  if (!draft) return;
  const sheet = sheets.value.find((item) => item.name === draft.sheetName);
  if (sheet) applyHeaders(sheet.headers);
}

async function importWorkbook() {
  const draft = templateDraft.value;
  if (!draft) return;
  try {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Excel 工作簿", extensions: ["xlsx"] }],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    const slash = path.lastIndexOf("/");
    draft.targetPath = slash > 0 ? path.slice(0, slash) : draft.targetPath;
    draft.name = path
      .slice(slash + 1)
      .replace(/\.xlsx$/i, "")
      .trim();
    await loadSheets(true);
    notify("已导入表格列定义");
  } catch (error) {
    notify(String(error), "error");
  }
}

function cloneTemplate(template: Template): Template {
  return {
    ...template,
    columns: template.columns.map((column) => ({ ...column })),
  };
}

function selectTemplate(template: Template) {
  templateDraft.value = cloneTemplate(template);
  templateDeleteId.value = null;
  sheets.value = [];
  void loadSheets(false);
}

function onTemplateSelect(event: Event) {
  const id = (event.target as HTMLSelectElement).value;
  const found = templates.value.find((item) => item.id === id);
  if (found) selectTemplate(found);
}

function newTemplate() {
  const base = templateDraft.value;
  templateDraft.value = {
    id: "",
    name: "新配置",
    isDefault: false,
    columns: base
      ? base.columns.map((column) => ({ ...column }))
      : [{ key: "hospital", label: "医院", width: 24, format: "text", hint: "" }],
    targetPath: "",
    sheetName: "sheet1",
    headerRow: 1,
    fileNameRule: "默认导出.xlsx",
  };
  templateDeleteId.value = null;
}

function duplicateTemplate() {
  const base = templateDraft.value;
  if (!base) return;
  templateDraft.value = {
    ...cloneTemplate(base),
    id: "",
    name: `${base.name} 副本`,
    isDefault: false,
  };
  templateDeleteId.value = null;
}

function addTemplateColumn() {
  const draft = templateDraft.value;
  if (!draft) return;
  draft.columns.push({
    key: `col_${draft.columns.length + 1}`,
    label: "",
    width: 16,
    format: "text",
    hint: "",
  });
}

function removeTemplateColumn(index: number) {
  templateDraft.value?.columns.splice(index, 1);
}

async function loadTemplates() {
  try {
    templates.value = await api.listTemplates();
  } catch (error) {
    notify(String(error), "error");
  }
}

async function pickTarget() {
  const draft = templateDraft.value;
  if (!draft) return;
  try {
    const picked = await open({
      multiple: false,
      directory: true,
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (path) {
      draft.targetPath = path;
      await loadSheets(true);
    }
  } catch (error) {
    notify(String(error), "error");
  }
}

async function saveTemplateDraft() {
  const draft = templateDraft.value;
  if (!draft) return;
  if (!draft.name.trim()) {
    notify("配置名称不能为空", "error");
    return;
  }
  const columns = draft.columns.filter((column) => column.label.trim());
  columns.forEach((column, index) => {
    if (!column.key.trim()) column.key = `col_${index + 1}`;
    column.width = Math.min(80, Math.max(6, Number(column.width) || 16));
  });
  savingTemplate.value = true;
  try {
    const saved = await api.saveTemplate({
      ...draft,
      columns,
      sheetName: draft.sheetName.trim() || draft.name.trim(),
    });
    await loadTemplates();
    selectTemplate(saved);
    emit("changed");
    notify("配置已保存");
  } catch (error) {
    notify(String(error), "error");
  } finally {
    savingTemplate.value = false;
  }
}

async function makeTemplateDefault() {
  const draft = templateDraft.value;
  if (!draft) return;
  try {
    let id = draft.id;
    if (!id) {
      const saved = await api.saveTemplate(draft);
      id = saved.id;
      templateDraft.value = cloneTemplate(saved);
    }
    await api.setDefaultTemplate(id);
    await loadTemplates();
    if (templateDraft.value) templateDraft.value.isDefault = true;
    emit("changed");
    notify("已设为默认配置");
  } catch (error) {
    notify(String(error), "error");
  }
}

async function removeTemplate() {
  const draft = templateDraft.value;
  if (!draft?.id) return;
  if (templateDeleteId.value !== draft.id) {
    templateDeleteId.value = draft.id;
    window.clearTimeout(templateDeleteTimer);
    templateDeleteTimer = window.setTimeout(() => {
      templateDeleteId.value = null;
    }, 3000);
    return;
  }
  templateDeleteId.value = null;
  try {
    await api.deleteTemplate(draft.id);
    await loadTemplates();
    const next = templates.value[0];
    templateDraft.value = next ? cloneTemplate(next) : null;
    emit("changed");
    notify("配置已删除");
  } catch (error) {
    notify(String(error), "error");
  }
}

onMounted(async () => {
  try {
    await loadTemplates();
    const preferred =
      templates.value.find((item) => item.isDefault) ?? templates.value[0];
    if (preferred) selectTemplate(preferred);
  } catch (error) {
    notify(String(error), "error");
  }
});

onBeforeUnmount(() => {
  columnSortable?.destroy();
  columnSortable = null;
  window.clearTimeout(templateDeleteTimer);
});
</script>

<template>
  <div class="export-panel">
    <div v-if="!templateDraft" class="settings-row">
      <div class="settings-label">配置</div>
      <div class="settings-control">
        <button class="btn small" @click="newTemplate">新建配置</button>
      </div>
    </div>

    <div v-if="templateDraft" class="export-fields">
      <div class="settings-row">
        <div class="settings-label">配置名称</div>
        <div class="settings-control">
          <div class="inline-row export-name-row">
            <input v-model="templateDraft.name" class="field-input export-name-input" />
            <select
              :value="templateDraft.id"
              class="field-input export-template-select"
              @change="onTemplateSelect"
            >
              <option v-for="item in templates" :key="item.id" :value="item.id">
                {{ item.name }}{{ item.isDefault ? "（默认）" : "" }}
              </option>
            </select>
            <button class="btn small" @click="newTemplate">新建</button>
            <button class="btn small" @click="duplicateTemplate">复制</button>
            <button class="text-btn" :disabled="!templateDraft.id" @click="removeTemplate">
              {{ templateDeleteId === templateDraft.id ? "确认删除" : "删除" }}
            </button>
          </div>
        </div>
      </div>
      <div class="settings-row">
        <div class="settings-label">目标目录</div>
        <div class="settings-control">
          <div class="inline-row">
            <input
              v-model="templateDraft.targetPath"
              class="field-input"
              placeholder="选择导出目录"
            />
            <button class="btn small" @click="pickTarget">选择目录</button>
            <button class="btn small" @click="importWorkbook">导入表格</button>
          </div>
        </div>
      </div>
      <div class="settings-row">
        <div class="settings-label">工作表</div>
        <div class="settings-control">
          <div class="inline-row">
            <select
              v-if="sheets.length > 0"
              v-model="templateDraft.sheetName"
              class="field-input"
              @change="onSheetChange"
            >
              <option v-for="sheet in sheets" :key="sheet.name" :value="sheet.name">
                {{ sheet.name }}
              </option>
            </select>
            <input v-else v-model="templateDraft.sheetName" class="field-input" />
          </div>
        </div>
      </div>
      <div class="settings-row">
        <div class="settings-label">列定义</div>
        <div class="settings-control">
          <div class="template-col-row head">
            <span></span>
            <span>列名</span>
            <span>示例</span>
            <span>列宽</span>
            <span>格式</span>
            <span></span>
          </div>
          <div ref="columnListRef" class="template-col-list">
            <div
              v-for="(column, index) in templateDraft.columns"
              :key="column.key || index"
              class="template-col-row"
            >
              <span class="drag-handle">⠿</span>
              <input v-model="column.label" class="field-input" />
              <input v-model="column.hint" class="field-input" placeholder="示例值" />
              <input
                v-model.number="column.width"
                class="field-input"
                type="number"
                min="6"
                max="80"
              />
              <select v-model="column.format" class="field-input">
                <option value="text">文本</option>
                <option value="number">数字</option>
                <option value="date">日期</option>
              </select>
              <div class="row-actions">
                <button class="text-btn" @click="removeTemplateColumn(index)">删除</button>
              </div>
            </div>
          </div>
          <div class="row-actions template-actions">
            <button class="btn small" @click="addTemplateColumn">新增列</button>
            <div class="template-actions-right">
              <button class="text-btn" @click="makeTemplateDefault">设为默认</button>
              <button
                class="btn primary small"
                :disabled="savingTemplate"
                @click="saveTemplateDraft"
              >
                {{ savingTemplate ? "保存中…" : "保存配置" }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

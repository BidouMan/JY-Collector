<script setup lang="ts">
import { open } from "@tauri-apps/plugin-dialog";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import Sortable from "sortablejs";
import { api } from "../api";
import DropdownSelect from "./DropdownSelect.vue";
import { notify } from "../toast";
import type { SheetInfo, Template, TemplateColumn, TemplateSheet } from "../types";

const emit = defineEmits<{ changed: [] }>();

const templates = ref<Template[]>([]);
const templateDraft = ref<Template | null>(null);
const savingTemplate = ref(false);
const importing = ref(false);
const templateDeleteId = ref<string | null>(null);
const activeSheetIndex = ref(0);
const templateOptions = computed(() =>
  templates.value.map((item) => ({
    value: item.id,
    label: `${item.name}${item.isDefault ? "（默认）" : ""}`,
  })),
);
const activeSheet = computed<TemplateSheet | null>(
  () => templateDraft.value?.sheets[activeSheetIndex.value] ?? null,
);
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
      const columns = activeSheet.value?.columns;
      if (!columns) return;
      const { oldIndex, newIndex } = event;
      if (oldIndex == null || newIndex == null || oldIndex === newIndex) return;
      const [moved] = columns.splice(oldIndex, 1);
      if (moved) columns.splice(newIndex, 0, moved);
    },
  });
}

watch([columnListRef, activeSheetIndex], initColumnSortable, { flush: "post" });

function workbookFile(template: Template): string {
  const raw = template.targetPath.trim();
  if (/\.xlsx$/i.test(raw)) return raw;
  const dir = raw.replace(/\/+$/, "");
  const name = (template.name.trim() || "默认导出").replace(/[\/\\:*?"<>|]/g, "_");
  const fileName = `${name}.xlsx`;
  return dir ? `${dir}/${fileName}` : fileName;
}

function defaultColumn(index: number): TemplateColumn {
  return {
    key: `col_${index + 1}`,
    label: "",
    width: 18,
    format: "text",
    hint: "",
  };
}

function cloneSheet(sheet: TemplateSheet): TemplateSheet {
  return {
    ...sheet,
    columns: sheet.columns.map((column) => ({ ...column })),
  };
}

function cloneTemplate(template: Template): Template {
  return {
    ...template,
    columns: template.columns.map((column) => ({ ...column })),
    sheets: (template.sheets ?? []).map(cloneSheet),
  };
}

/** 老模板没有 sheets 时，用 sheetName + columns 兜成单工作表。 */
function normalizeDraft(raw: Template): Template {
  const draft = cloneTemplate(raw);
  if (draft.sheets.length === 0) {
    draft.sheets = [
      {
        name: raw.sheetName.trim() || "sheet1",
        headerRow: raw.headerRow || 1,
        columns: draft.columns.map((column) => ({ ...column })),
        primary: true,
      },
    ];
  }
  let primarySeen = false;
  for (const sheet of draft.sheets) {
    if (sheet.primary) {
      if (primarySeen) sheet.primary = false;
      else primarySeen = true;
    }
    sheet.headerRow = Math.max(1, Number(sheet.headerRow) || 1);
  }
  if (!primarySeen) draft.sheets[0].primary = true;
  return draft;
}

function syncLegacy(draft: Template) {
  const primary = draft.sheets.find((sheet) => sheet.primary) ?? draft.sheets[0];
  if (!primary) return;
  draft.columns = primary.columns.map((column) => ({ ...column }));
  draft.sheetName = primary.name;
  draft.headerRow = primary.headerRow;
}

function columnsFromHeaders(
  headers: string[],
  previous: TemplateColumn[] | undefined,
): TemplateColumn[] {
  return headers.map((label, index) => {
    const existing = previous?.find((column) => column.label === label);
    return {
      key: existing?.key || `col_${index + 1}`,
      label,
      width: existing?.width ?? 18,
      format: existing?.format ?? "text",
      hint: existing?.hint ?? "",
      options: existing?.options ?? [],
    };
  });
}

/** 用工作簿里的所有 sheet 重建草稿；同名 sheet 的列设置尽量保留。 */
function applyWorkbookSheets(list: SheetInfo[]) {
  const draft = templateDraft.value;
  if (!draft || list.length === 0) return;
  draft.sheets = list.map((sheet, index) => {
    const previous = draft.sheets.find((item) => item.name === sheet.name);
    return {
      name: sheet.name,
      headerRow: previous?.headerRow ?? 1,
      columns: columnsFromHeaders(sheet.headers, previous?.columns),
      primary: index === 0,
    };
  });
  activeSheetIndex.value = 0;
  syncLegacy(draft);
}

async function adoptWorkbook(path: string) {
  try {
    const list = await api.readWorkbook(path);
    applyWorkbookSheets(list);
  } catch (error) {
    notify(String(error), "error");
  }
}

async function importWorkbook() {
  const draft = templateDraft.value;
  if (!draft) return;
  try {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Excel 工作簿", extensions: ["xlsx"] }],
    });
    const source = Array.isArray(picked) ? picked[0] : picked;
    if (!source) return;
    importing.value = true;
    const copied = await api.importWorkbook(source);
    draft.targetPath = copied;
    const file = copied.split(/[\\/]/).pop() ?? copied;
    draft.name = file.replace(/\.xlsx$/i, "").trim();
    await adoptWorkbook(copied);
    notify(`已导入 ${draft.sheets.length} 个工作表`);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    importing.value = false;
  }
}

function selectTemplate(template: Template) {
  templateDraft.value = normalizeDraft(template);
  templateDeleteId.value = null;
  activeSheetIndex.value = 0;
}

function onTemplateSelect(id: string) {
  const found = templates.value.find((item) => item.id === id);
  if (found) selectTemplate(found);
}

function newTemplate() {
  const base = templateDraft.value;
  const columns = base
    ? base.columns.map((column) => ({ ...column }))
    : [defaultColumn(0)];
  templateDraft.value = {
    id: "",
    name: "新配置",
    isDefault: false,
    columns,
    targetPath: "",
    sheetName: base?.sheetName ?? "sheet1",
    headerRow: base?.headerRow ?? 1,
    fileNameRule: "默认导出.xlsx",
    sheets: [
      {
        name: base?.sheetName ?? "sheet1",
        headerRow: base?.headerRow ?? 1,
        columns,
        primary: true,
      },
    ],
  };
  activeSheetIndex.value = 0;
  templateDeleteId.value = null;
}

function duplicateTemplate() {
  const base = templateDraft.value;
  if (!base) return;
  const draft = cloneTemplate(base);
  draft.id = "";
  draft.name = `${base.name} 副本`;
  draft.isDefault = false;
  templateDraft.value = draft;
  activeSheetIndex.value = 0;
  templateDeleteId.value = null;
}

function addSheet() {
  const draft = templateDraft.value;
  if (!draft) return;
  const used = new Set(draft.sheets.map((sheet) => sheet.name));
  let index = draft.sheets.length + 1;
  let name = `Sheet${index}`;
  while (used.has(name)) {
    index += 1;
    name = `Sheet${index}`;
  }
  draft.sheets.push({
    name,
    headerRow: 1,
    columns: [defaultColumn(0)],
    primary: false,
  });
  activeSheetIndex.value = draft.sheets.length - 1;
}

function removeSheet(index: number) {
  const draft = templateDraft.value;
  if (!draft) return;
  if (draft.sheets.length <= 1) {
    notify("至少保留一个工作表", "error");
    return;
  }
  draft.sheets.splice(index, 1);
  if (!draft.sheets.some((sheet) => sheet.primary)) {
    draft.sheets[0].primary = true;
  }
  activeSheetIndex.value = Math.min(activeSheetIndex.value, draft.sheets.length - 1);
  syncLegacy(draft);
}

function setPrimary(index: number) {
  const draft = templateDraft.value;
  if (!draft) return;
  draft.sheets.forEach((sheet, position) => {
    sheet.primary = position === index;
  });
  syncLegacy(draft);
}

function addTemplateColumn() {
  const sheet = activeSheet.value;
  if (!sheet) return;
  sheet.columns.push(defaultColumn(sheet.columns.length));
}

function removeTemplateColumn(index: number) {
  activeSheet.value?.columns.splice(index, 1);
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
    if (!path) return;
    draft.targetPath = path;
    await adoptWorkbook(workbookFile(draft));
  } catch (error) {
    notify(String(error), "error");
  }
}

/** 把草稿规范化成可保存的模板；校验不过时返回 null。 */
function buildPayload(): Template | null {
  const draft = templateDraft.value;
  if (!draft) return null;
  if (!draft.name.trim()) {
    notify("配置名称不能为空", "error");
    return null;
  }
  if (draft.sheets.length === 0) {
    notify("至少保留一个工作表", "error");
    return null;
  }

  const sheets: TemplateSheet[] = draft.sheets.map((sheet, index) => {
    const used = new Set<string>();
    const columns = sheet.columns
      .filter((column) => column.label.trim())
      .map((column, columnIndex) => {
        let key = column.key.trim();
        if (!key || used.has(key)) {
          let counter = columnIndex + 1;
          while (used.has(`col_${counter}`)) counter += 1;
          key = `col_${counter}`;
        }
        used.add(key);
        return {
          key,
          label: column.label.trim(),
          width: Math.min(80, Math.max(6, Number(column.width) || 18)),
          format: column.format,
          hint: column.hint.trim(),
          options: column.options ?? [],
        };
      });
    return {
      name: sheet.name.trim() || `Sheet${index + 1}`,
      headerRow: Math.max(1, Number(sheet.headerRow) || 1),
      columns,
      primary: sheet.primary,
    };
  });

  const primary = sheets.find((sheet) => sheet.primary) ?? sheets[0];
  sheets.forEach((sheet) => {
    sheet.primary = sheet === primary;
  });

  return {
    ...draft,
    sheets,
    columns: primary.columns.map((column) => ({ ...column })),
    sheetName: primary.name,
    headerRow: primary.headerRow,
  };
}

async function saveTemplateDraft() {
  const payload = buildPayload();
  if (!payload) return;
  savingTemplate.value = true;
  try {
    const saved = await api.saveTemplate(payload);
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
      const payload = buildPayload();
      if (!payload) return;
      const saved = await api.saveTemplate(payload);
      id = saved.id;
      templateDraft.value = normalizeDraft(saved);
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
    templateDraft.value = next ? normalizeDraft(next) : null;
    activeSheetIndex.value = 0;
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
            <DropdownSelect
              :model-value="templateDraft.id"
              :options="templateOptions"
              class="export-template-select"
              @change="onTemplateSelect"
            />
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
              placeholder="选择导出目录，或上传一个 Excel"
            />
            <button class="btn small" @click="pickTarget">选择目录</button>
            <button class="btn small" :disabled="importing" @click="importWorkbook">
              {{ importing ? "上传中…" : "上传 Excel" }}
            </button>
          </div>
          <div class="hint">
            上传会把 Excel 复制到应用数据目录，并按里面的工作表建立分类。
          </div>
        </div>
      </div>

      <div class="settings-row">
        <div class="settings-label">工作表</div>
        <div class="settings-control">
          <div class="sheet-editor">
            <div class="sheet-tabs">
              <button
                v-for="(sheet, index) in templateDraft.sheets"
                :key="index"
                class="sheet-tab"
                :class="{ active: index === activeSheetIndex }"
                @click="activeSheetIndex = index"
              >
                <span class="sheet-tab-name">
                  {{ sheet.name.trim() || `Sheet${index + 1}` }}
                </span>
                <span v-if="sheet.primary" class="sheet-primary-tag">录入</span>
              </button>
              <button
                class="sheet-add"
                type="button"
                title="新建工作表"
                aria-label="新建工作表"
                @click="addSheet"
              >
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <path d="M8 3v10M3 8h10" />
                </svg>
                <span>新建工作表</span>
              </button>
            </div>

            <div v-if="activeSheet" class="sheet-meta">
              <label class="sheet-field">
                <span>名称</span>
                <input v-model="activeSheet.name" class="field-input" />
              </label>
              <label class="sheet-field sheet-field-narrow">
                <span>表头行</span>
                <input
                  v-model.number="activeSheet.headerRow"
                  class="field-input"
                  type="number"
                  min="1"
                />
              </label>
              <label class="required-toggle">
                <input
                  type="checkbox"
                  :checked="activeSheet.primary"
                  @change="setPrimary(activeSheetIndex)"
                />
                录入 / 导出用
              </label>
              <button
                class="text-btn"
                :disabled="templateDraft.sheets.length <= 1"
                @click="removeSheet(activeSheetIndex)"
              >
                删除工作表
              </button>
            </div>
          </div>
        </div>
      </div>

      <div class="settings-row">
        <div class="settings-label">列定义</div>
        <div class="settings-control">
          <div class="hint">
            当前工作表：{{ activeSheet?.name || "未选择" }}。列名必填，示例可留空。
          </div>
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
              v-for="(column, index) in activeSheet?.columns ?? []"
              :key="`${activeSheetIndex}-${column.key}-${index}`"
              class="template-col-row"
            >
              <span class="drag-handle">⠿</span>
              <input v-model="column.label" class="field-input" placeholder="列名" />
              <input v-model="column.hint" class="field-input" placeholder="示例值（可留空）" />
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
                <option value="select">单选</option>
                <option value="multi_select">多选</option>
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

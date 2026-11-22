<script setup lang="ts">
import { computed, ref } from "vue";
import DropdownSelect from "./DropdownSelect.vue";
import type {
  TableViewConfig,
  TableViewFilter,
  TableViewGroup,
  TableViewSort,
} from "../types";

type Panel = "filter" | "sort" | "group";

const props = defineProps<{
  headers: string[];
  keys: string[];
  formats?: string[];
  config: TableViewConfig;
}>();

const emit = defineEmits<{
  (event: "change", patch: Partial<TableViewConfig>): void;
}>();

const panel = ref<Panel | null>(null);
const panelStyle = ref<Record<string, string>>({});

const fieldOptions = computed(() =>
  props.keys
    .map((key, index) => ({
      value: key,
      label: props.headers[index] || key,
    }))
    .filter((option) => option.value),
);

const operatorOptions = [
  { value: "contains", label: "包含" },
  { value: "equals", label: "等于" },
  { value: "empty", label: "为空" },
  { value: "notEmpty", label: "不为空" },
  { value: "gt", label: "大于" },
  { value: "lt", label: "小于" },
];

const directionOptions = [
  { value: "asc", label: "升序" },
  { value: "desc", label: "降序" },
];

const filters = computed(() => props.config.filters ?? []);
const sorts = computed(() => props.config.sorts ?? []);
const groups = computed(() => props.config.groupKeys ?? []);
const filterLogic = computed(() => (props.config.filterLogic === "or" ? "or" : "and"));

const panelTitle = computed(() => {
  if (panel.value === "filter") return "筛选条件";
  if (panel.value === "sort") return "排序规则";
  if (panel.value === "group") return "分组规则";
  return "";
});

function formatOf(key: string): string {
  const index = props.keys.indexOf(key);
  return props.formats?.[index] ?? "text";
}

function inputType(key: string): string {
  const format = formatOf(key);
  if (format === "number") return "number";
  if (format === "date") return "date";
  return "text";
}

function togglePanel(event: MouseEvent, next: Panel) {
  if (panel.value === next) {
    panel.value = null;
    return;
  }
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  const width = 460;
  panelStyle.value = {
    position: "fixed",
    top: `${Math.round(rect.bottom + 6)}px`,
    left: `${Math.max(12, Math.min(rect.left, window.innerWidth - width - 12))}px`,
    width: `${width}px`,
  };
  panel.value = next;
}

function closePanel() {
  panel.value = null;
}

function setFilters(next: TableViewFilter[]) {
  emit("change", { filters: next });
}

function addFilter() {
  setFilters([
    ...filters.value,
    {
      key: fieldOptions.value[0]?.value ?? "",
      operator: "contains",
      value: "",
    },
  ]);
}

function patchFilter(index: number, patch: Partial<TableViewFilter>) {
  setFilters(
    filters.value.map((filter, position) =>
      position === index ? { ...filter, ...patch } : filter,
    ),
  );
}

function removeFilter(index: number) {
  setFilters(filters.value.filter((_, position) => position !== index));
}

function setFilterLogic(logic: "and" | "or") {
  emit("change", { filterLogic: logic });
}

function setSorts(next: TableViewSort[]) {
  emit("change", { sorts: next });
}

function addSort() {
  setSorts([
    ...sorts.value,
    {
      key: fieldOptions.value[0]?.value ?? "",
      direction: "asc",
    },
  ]);
}

function patchSort(index: number, patch: Partial<TableViewSort>) {
  setSorts(
    sorts.value.map((sort, position) =>
      position === index ? { ...sort, ...patch } : sort,
    ),
  );
}

function moveSort(index: number, delta: number) {
  const target = index + delta;
  if (target < 0 || target >= sorts.value.length) return;
  const next = [...sorts.value];
  const [item] = next.splice(index, 1);
  next.splice(target, 0, item);
  setSorts(next);
}

function removeSort(index: number) {
  setSorts(sorts.value.filter((_, position) => position !== index));
}

function setGroups(next: TableViewGroup[]) {
  emit("change", { groupKeys: next });
}

function addGroup() {
  setGroups([
    ...groups.value,
    {
      key: fieldOptions.value[0]?.value ?? "",
      direction: "asc",
    },
  ]);
}

function patchGroup(index: number, patch: Partial<TableViewGroup>) {
  setGroups(
    groups.value.map((group, position) =>
      position === index ? { ...group, ...patch } : group,
    ),
  );
}

function moveGroup(index: number, delta: number) {
  const target = index + delta;
  if (target < 0 || target >= groups.value.length) return;
  const next = [...groups.value];
  const [item] = next.splice(index, 1);
  next.splice(target, 0, item);
  setGroups(next);
}

function removeGroup(index: number) {
  setGroups(groups.value.filter((_, position) => position !== index));
}

function clearCurrent() {
  if (panel.value === "filter") setFilters([]);
  else if (panel.value === "sort") setSorts([]);
  else if (panel.value === "group") setGroups([]);
}

function clearFilters() {
  setFilters([]);
}

function clearSorts() {
  setSorts([]);
}

function clearGroups() {
  setGroups([]);
}
</script>

<template>
  <div class="view-rules-bar">
    <div class="rule-buttons">
      <button
        class="btn small"
        :class="{ active: filters.length > 0 }"
        @click="togglePanel($event, 'filter')"
      >
        筛选
        <span v-if="filters.length" class="rule-count">{{ filters.length }}</span>
      </button>
      <button
        class="btn small"
        :class="{ active: sorts.length > 0 }"
        @click="togglePanel($event, 'sort')"
      >
        排序
        <span v-if="sorts.length" class="rule-count">{{ sorts.length }}</span>
      </button>
      <button
        class="btn small"
        :class="{ active: groups.length > 0 }"
        @click="togglePanel($event, 'group')"
      >
        分组
        <span v-if="groups.length" class="rule-count">{{ groups.length }}</span>
      </button>
    </div>
  </div>

  <Teleport to="body">
    <div v-if="panel" class="rule-popover-backdrop" @click="closePanel">
      <div class="rule-popover" :style="panelStyle" @click.stop>
        <div class="rule-head">
          <span class="rule-title">{{ panelTitle }}</span>
          <div class="rule-head-actions">
            <button class="text-btn" @click="clearCurrent">清空</button>
            <button class="text-btn" @click="closePanel">完成</button>
          </div>
        </div>

        <template v-if="panel === 'filter'">
          <div class="rule-logic">
            <button
              class="rule-logic-item"
              :class="{ active: filterLogic === 'and' }"
              @click="setFilterLogic('and')"
            >
              满足所有条件
            </button>
            <button
              class="rule-logic-item"
              :class="{ active: filterLogic === 'or' }"
              @click="setFilterLogic('or')"
            >
              满足任一条件
            </button>
          </div>

          <div v-if="filters.length === 0" class="rule-empty">还没有筛选条件</div>
          <div
            v-for="(filter, index) in filters"
            :key="`filter-${index}`"
            class="rule-row"
          >
            <DropdownSelect
              :model-value="filter.key"
              :options="fieldOptions"
              @change="(value) => patchFilter(index, { key: value })"
            />
            <DropdownSelect
              :model-value="filter.operator"
              :options="operatorOptions"
              @change="
                (value) =>
                  patchFilter(index, {
                    operator: value as TableViewFilter['operator'],
                  })
              "
            />
            <input
              v-if="filter.operator !== 'empty' && filter.operator !== 'notEmpty'"
              class="field-input rule-value"
              :type="inputType(filter.key)"
              :value="filter.value"
              placeholder="值"
              @input="
                patchFilter(index, {
                  value: ($event.target as HTMLInputElement).value,
                })
              "
            />
            <button class="rule-icon" title="删除条件" @click="removeFilter(index)">
              ×
            </button>
          </div>
          <button class="rule-add" @click="addFilter">＋ 添加条件</button>
          <div v-if="filters.length" class="rule-foot">
            <button class="text-btn" @click="clearFilters">清除全部筛选</button>
          </div>
        </template>

        <template v-else-if="panel === 'sort'">
          <div v-if="sorts.length === 0" class="rule-empty">还没有排序规则</div>
          <div
            v-for="(sort, index) in sorts"
            :key="`sort-${index}`"
            class="rule-row"
          >
            <span class="rule-order">{{ index + 1 }}</span>
            <DropdownSelect
              :model-value="sort.key"
              :options="fieldOptions"
              @change="(value) => patchSort(index, { key: value })"
            />
            <DropdownSelect
              :model-value="sort.direction"
              :options="directionOptions"
              @change="(value) => patchSort(index, { direction: value as 'asc' | 'desc' })"
            />
            <button
              class="rule-icon"
              title="上移"
              :disabled="index === 0"
              @click="moveSort(index, -1)"
            >
              ↑
            </button>
            <button
              class="rule-icon"
              title="下移"
              :disabled="index === sorts.length - 1"
              @click="moveSort(index, 1)"
            >
              ↓
            </button>
            <button class="rule-icon" title="删除条件" @click="removeSort(index)">
              ×
            </button>
          </div>
          <button class="rule-add" @click="addSort">＋ 添加排序</button>
          <div v-if="sorts.length" class="rule-foot">
            <button class="text-btn" @click="clearSorts">清除全部排序</button>
          </div>
        </template>

        <template v-else-if="panel === 'group'">
          <div v-if="groups.length === 0" class="rule-empty">还没有分组规则</div>
          <div
            v-for="(group, index) in groups"
            :key="`group-${index}`"
            class="rule-row"
          >
            <span class="rule-order">{{ index + 1 }}</span>
            <DropdownSelect
              :model-value="group.key"
              :options="fieldOptions"
              @change="(value) => patchGroup(index, { key: value })"
            />
            <DropdownSelect
              :model-value="group.direction"
              :options="directionOptions"
              @change="
                (value) => patchGroup(index, { direction: value as 'asc' | 'desc' })
              "
            />
            <button
              class="rule-icon"
              title="上移"
              :disabled="index === 0"
              @click="moveGroup(index, -1)"
            >
              ↑
            </button>
            <button
              class="rule-icon"
              title="下移"
              :disabled="index === groups.length - 1"
              @click="moveGroup(index, 1)"
            >
              ↓
            </button>
            <button class="rule-icon" title="删除条件" @click="removeGroup(index)">
              ×
            </button>
          </div>
          <button class="rule-add" @click="addGroup">＋ 添加分组</button>
          <div v-if="groups.length" class="rule-foot">
            <button class="text-btn" @click="clearGroups">清除全部分组</button>
          </div>
        </template>
      </div>
    </div>
  </Teleport>
</template>

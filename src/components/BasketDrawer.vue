<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { api } from "../api";
import ExportSettingsPanel from "./ExportSettingsPanel.vue";
import { appendSummary, ensureTargetPath, generateRecords } from "../services/ledger";
import { basket, clearBasketNow, removeFromBasket } from "../stores/basket";
import { notify } from "../toast";
import type { Template } from "../types";
import { formatTime } from "../utils";
import { replaceWechatEmoji } from "../wechatEmoji";

const emit = defineEmits<{ changed: [] }>();

const templates = ref<Template[]>([]);
const templateId = ref("");
const groupFilter = ref("");
const personFilter = ref("");
const keyword = ref("");
const busy = ref(false);
const exportOpen = ref(false);

const groups = computed(() => {
  const map = new Map<string, { id: string; name: string; count: number }>();
  for (const item of basket.items) {
    const entry = map.get(item.groupId) ?? {
      id: item.groupId,
      name: item.groupName,
      count: 0,
    };
    entry.count += 1;
    map.set(item.groupId, entry);
  }
  return [...map.values()].sort((a, b) => b.count - a.count);
});

const people = computed(() => {
  const set = new Set(basket.items.map((item) => item.senderName));
  return [...set].sort((a, b) => a.localeCompare(b, "zh-CN"));
});

const stats = computed(() => `${basket.items.length} 条 · ${groups.value.length} 个群`);

const filteredItems = computed(() => {
  const query = keyword.value.trim().toLowerCase();
  return basket.items.filter((item) => {
    if (groupFilter.value && item.groupId !== groupFilter.value) return false;
    if (personFilter.value && item.senderName !== personFilter.value) return false;
    if (query) {
      const text = `${item.content} ${item.senderName} ${item.groupName}`.toLowerCase();
      if (!text.includes(query)) return false;
    }
    return true;
  });
});

const template = computed(
  () => templates.value.find((item) => item.id === templateId.value) ?? templates.value[0],
);

async function loadTemplates() {
  try {
    templates.value = await api.listTemplates();
    const exists = templates.value.some((item) => item.id === templateId.value);
    if (!exists) {
      const preferred = templates.value.find((item) => item.isDefault) ?? templates.value[0];
      templateId.value = preferred?.id ?? "";
    }
  } catch (error) {
    notify(String(error), "error");
  }
}

function toggleGroup(id: string) {
  groupFilter.value = groupFilter.value === id ? "" : id;
}

async function remove(id: string) {
  try {
    await removeFromBasket([id]);
  } catch (error) {
    notify(String(error), "error");
  }
}

async function clear() {
  try {
    await clearBasketNow();
    groupFilter.value = "";
    personFilter.value = "";
  } catch (error) {
    notify(String(error), "error");
  }
}

async function run(append: boolean) {
  const tpl = template.value;
  if (!tpl) {
    notify("没有可用的导出配置", "error");
    return;
  }
  const items = filteredItems.value;
  if (!items.length) {
    notify("还没有选中的消息", "error");
    return;
  }
  busy.value = true;
  try {
    const { ids, rows, engine, warning } = await generateRecords(items, tpl);
    if (!rows) {
      notify(warning || "没有可生成的记录", "error");
      return;
    }
    emit("changed");
    if (!append) {
      notify(engine === "model" ? `模型已生成 ${rows} 条记录` : `已生成 ${rows} 条记录`);
      if (warning) notify(warning, "error");
      return;
    }
    const path = await ensureTargetPath(tpl);
    if (!path) return;
    const report = await api.appendLedger(ids, tpl.id, path, false);
    notify(appendSummary(report));
    if (warning) notify(warning, "error");
    emit("changed");
  } catch (error) {
    notify(String(error), "error");
  } finally {
    busy.value = false;
  }
}

watch(
  () => basket.open,
  (open) => {
    if (open && templates.value.length === 0) void loadTemplates();
  },
);

if (basket.open && templates.value.length === 0) void loadTemplates();
</script>

<template>
  <div v-if="basket.open" class="basket-backdrop" @click="basket.open = false"></div>

  <aside v-if="basket.open" class="basket-pane">
    <div class="basket-head">
      <div class="record-title-row">
        <span class="record-title">已选消息</span>
        <span class="badge">{{ stats }}</span>
      </div>
      <div class="basket-head-actions">
        <button class="text-btn" @click="exportOpen = true">导出设置</button>
        <button class="text-btn" @click="basket.open = false">关闭</button>
      </div>
    </div>

    <div class="basket-dist">
      <button
        v-for="group in groups"
        :key="group.id"
        class="chip"
        :class="{ active: groupFilter === group.id }"
        @click="toggleGroup(group.id)"
      >
        {{ group.name }} {{ group.count }}
      </button>
      <span v-if="groups.length === 0" class="muted">还没有选中的消息</span>
    </div>

    <div class="basket-filters">
      <select v-model="personFilter" class="field-input">
        <option value="">全部发送人</option>
        <option v-for="name in people" :key="name" :value="name">{{ name }}</option>
      </select>
      <input v-model="keyword" class="search-input" placeholder="筛选内容" />
    </div>

    <div class="basket-list">
      <div v-for="item in filteredItems" :key="item.id" class="basket-item">
        <div class="basket-item-top">
          <span class="sender">{{ item.senderName }}</span>
          <span class="basket-meta">
            {{ item.groupName }} · {{ formatTime(item.createdAt) }}
          </span>
          <button class="text-btn" @click="remove(item.id)">移除</button>
        </div>
        <div class="basket-item-text">{{ replaceWechatEmoji(item.content) }}</div>
      </div>
      <div v-if="filteredItems.length === 0" class="empty">
        <div class="empty-title">没有匹配的消息</div>
      </div>
    </div>

    <div class="basket-foot">
      <select v-model="templateId" class="field-input">
        <option v-for="item in templates" :key="item.id" :value="item.id">
          {{ item.name }}
        </option>
      </select>
      <div class="basket-actions">
        <button class="text-btn" :disabled="busy || !filteredItems.length" @click="clear">
          清空
        </button>
        <button class="text-btn" :disabled="busy || !filteredItems.length" @click="run(false)">
          生成记录
        </button>
        <button class="btn primary" :disabled="busy || !filteredItems.length" @click="run(true)">
          {{ busy ? "处理中…" : "生成并追加" }}
        </button>
      </div>
    </div>
  </aside>

  <div v-if="exportOpen" class="modal-backdrop" @click.self="exportOpen = false">
    <div class="modal export-modal">
      <div class="modal-head">
        <div class="modal-title">导出设置</div>
        <button class="text-btn" @click="exportOpen = false">关闭</button>
      </div>
      <div class="modal-body">
        <ExportSettingsPanel @changed="loadTemplates" />
      </div>
    </div>
  </div>
</template>

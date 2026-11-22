<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api } from "../api";
import { notify } from "../toast";
import type { Group, Person } from "../types";

const emit = defineEmits<{ close: []; saved: [] }>();

const groups = ref<Group[]>([]);
const people = ref<Person[]>([]);
const groupIds = ref<string[]>([]);
const personIds = ref<string[]>([]);
const configured = ref(false);
const tab = ref<"group" | "person">("group");
const query = ref("");
const saving = ref(false);
const loading = ref(true);

const filteredGroups = computed(() => {
  const keyword = query.value.trim().toLowerCase();
  if (!keyword) return groups.value;
  return groups.value.filter((group) => group.name.toLowerCase().includes(keyword));
});

const filteredPeople = computed(() => {
  const keyword = query.value.trim().toLowerCase();
  if (!keyword) return people.value;
  return people.value.filter(
    (person) =>
      person.senderName.toLowerCase().includes(keyword) ||
      person.groupNames.toLowerCase().includes(keyword),
  );
});

const rows = computed(() => {
  if (tab.value === "group") {
    return filteredGroups.value.map((group) => ({
      key: group.id,
      name: group.name,
      meta: `${group.kind === "private" ? "单聊" : "群聊"} · ${group.messageCount} 条`,
    }));
  }
  return filteredPeople.value.map((person) => ({
    key: person.senderId,
    name: person.senderName,
    meta: `${person.messageCount} 条`,
  }));
});

function checked(id: string) {
  return (tab.value === "group" ? groupIds.value : personIds.value).includes(id);
}

function toggle(id: string) {
  if (tab.value === "group") {
    groupIds.value = groupIds.value.includes(id)
      ? groupIds.value.filter((item) => item !== id)
      : [...groupIds.value, id];
  } else {
    personIds.value = personIds.value.includes(id)
      ? personIds.value.filter((item) => item !== id)
      : [...personIds.value, id];
  }
}

function selectAll() {
  if (tab.value === "group") {
    groupIds.value = groups.value.map((group) => group.id);
  } else {
    personIds.value = people.value.map((person) => person.senderId);
  }
}

function clearAll() {
  if (tab.value === "group") {
    groupIds.value = [];
  } else {
    personIds.value = [];
  }
}

function switchTab(next: "group" | "person") {
  tab.value = next;
  query.value = "";
}

async function save() {
  saving.value = true;
  try {
    await api.saveCaptureScope(groupIds.value, personIds.value);
    window.dispatchEvent(new Event("jy:scope-changed"));
    notify("采集范围已保存");
    emit("saved");
    emit("close");
  } catch (error) {
    notify(String(error), "error");
  } finally {
    saving.value = false;
  }
}

onMounted(async () => {
  try {
    const scope = await api.getCaptureScope();
    groups.value = scope.groups;
    people.value = scope.people;
    groupIds.value = scope.configured
      ? scope.selectedGroups
      : scope.groups.map((group) => group.id);
    personIds.value = scope.configured
      ? scope.selectedPeople
      : scope.people.map((person) => person.senderId);
    configured.value = scope.configured;
  } catch (error) {
    notify(String(error), "error");
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="modal scope-modal">
      <div class="modal-head">
        <div class="modal-title">采集范围</div>
        <button class="text-btn" @click="emit('close')">关闭</button>
      </div>

      <div class="modal-body scope-body">
        <div class="scope-head">
          <div class="segmented compact">
            <button :class="{ active: tab === 'group' }" @click="switchTab('group')">
              会话 {{ groupIds.length }}
            </button>
            <button :class="{ active: tab === 'person' }" @click="switchTab('person')">
              人物 {{ personIds.length }}
            </button>
          </div>
          <input
            v-model="query"
            class="search-input scope-search"
            :placeholder="tab === 'group' ? '搜索会话' : '搜索人物'"
          />
        </div>

        <div class="scope-list">
          <label v-for="row in rows" :key="row.key" class="scope-row">
            <input type="checkbox" :checked="checked(row.key)" @change="toggle(row.key)" />
            <span class="scope-name">{{ row.name }}</span>
            <span class="scope-meta">{{ row.meta }}</span>
          </label>
          <div v-if="loading" class="empty">加载中</div>
          <div v-else-if="rows.length === 0" class="empty">没有匹配项</div>
        </div>

        <div class="hint">
          {{
            configured
              ? "会话控制左侧会话列表，人物控制「关注」里能看到的人。"
              : "保存后，会话控制左侧会话列表，人物控制「关注」里能看到的人；新出现的会话和人物需要回来勾选。"
          }}
        </div>
      </div>

      <div class="modal-foot scope-foot">
        <div class="scope-summary">
          已选 {{ groupIds.length }} 个会话 · {{ personIds.length }} 位人物
        </div>
        <div class="scope-actions">
          <button class="text-btn" @click="selectAll">全选</button>
          <button class="text-btn" @click="clearAll">清空</button>
          <button class="btn small" :disabled="saving" @click="save">
            {{ saving ? "保存中…" : "保存" }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

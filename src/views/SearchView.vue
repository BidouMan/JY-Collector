<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api } from "../api";
import BasketDrawer from "../components/BasketDrawer.vue";
import { basket, basketIdSet, refreshBasket, toggleBasket } from "../stores/basket";
import { notify } from "../toast";
import type {
  Group,
  KeywordMode,
  Message,
  MessageType,
  Person,
  SearchItem,
} from "../types";
import { formatTime, splitHighlight, TYPE_LABEL } from "../utils";

const LIMIT = 50;

const groups = ref<Group[]>([]);
const people = ref<Person[]>([]);

const keyword = ref("");
const keywordMode = ref<KeywordMode>("fuzzy");
const groupQuery = ref("");
const groupIds = ref<string[]>([]);
const senderInput = ref("");
const senderId = ref("");
const senderName = ref("");
const senderOpen = ref(false);
const mention = ref("");
const types = ref<MessageType[]>([]);
const startDate = ref("");
const endDate = ref("");
const inBasketOnly = ref(false);
const inRecordOnly = ref(false);
const sort = ref<"desc" | "asc">("desc");

const results = ref<SearchItem[]>([]);
const total = ref(0);
const page = ref(0);
const loading = ref(false);
const searched = ref(false);

const contextItems = ref<Message[]>([]);
const contextTarget = ref<Message | null>(null);
const contextLoading = ref(false);

let timer: number | undefined;
let requestSeq = 0;

const TYPE_ORDER: MessageType[] = ["text", "quote", "image", "file"];

const filteredGroups = computed(() => {
  const query = groupQuery.value.trim().toLowerCase();
  if (!query) return groups.value;
  return groups.value.filter((group) => group.name.toLowerCase().includes(query));
});

const senderMatches = computed(() => {
  const query = senderInput.value.trim().toLowerCase();
  return people.value
    .filter((person) => !query || person.senderName.toLowerCase().includes(query))
    .slice(0, 8);
});

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / LIMIT)));

const pageItems = computed(() => results.value);

const allSelected = computed(
  () =>
    pageItems.value.length > 0 &&
    pageItems.value.every((item) => basketIdSet.value.has(item.id)),
);

const contextIsInBasket = computed(
  () => !!contextTarget.value && basketIdSet.value.has(contextTarget.value.id),
);

function localDate(offsetDays = 0): string {
  const date = new Date();
  date.setDate(date.getDate() + offsetDays);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function quickRange(days: number) {
  startDate.value = days <= 0 ? localDate(0) : localDate(-(days - 1));
  endDate.value = localDate(0);
  schedule(true);
}

function isQuickRange(days: number) {
  return (
    startDate.value === (days <= 0 ? localDate(0) : localDate(-(days - 1))) &&
    endDate.value === localDate(0)
  );
}

function toggleGroup(id: string) {
  groupIds.value = groupIds.value.includes(id)
    ? groupIds.value.filter((item) => item !== id)
    : [...groupIds.value, id];
  schedule(true);
}

function toggleType(type: MessageType) {
  types.value = types.value.includes(type)
    ? types.value.filter((item) => item !== type)
    : [...types.value, type];
  schedule(true);
}

function toggleBasketOnly() {
  inBasketOnly.value = !inBasketOnly.value;
  schedule(true);
}

function toggleRecordOnly() {
  inRecordOnly.value = !inRecordOnly.value;
  schedule(true);
}

function pickSender(person: Person) {
  senderId.value = person.senderId;
  senderName.value = "";
  senderInput.value = person.senderName;
  senderOpen.value = false;
  schedule(true);
}

function updateSender(value: string) {
  senderInput.value = value;
  if (senderId.value) senderId.value = "";
  senderName.value = value;
  senderOpen.value = true;
  schedule(true);
}

async function runSearch() {
  const seq = ++requestSeq;
  loading.value = true;
  try {
    const data = await api.searchMessages({
      keyword: keyword.value,
      keywordMode: keywordMode.value,
      groupIds: groupIds.value,
      senderId: senderId.value,
      senderName: senderName.value,
      mention: mention.value,
      types: types.value,
      startDate: startDate.value,
      endDate: endDate.value,
      inBasket: inBasketOnly.value ? true : null,
      inRecord: inRecordOnly.value ? true : null,
      sort: sort.value,
      offset: page.value * LIMIT,
      limit: LIMIT,
    });
    if (seq !== requestSeq) return;
    results.value = data.items;
    total.value = data.total;
    if (page.value > 0 && page.value * LIMIT >= data.total) {
      page.value = Math.max(0, Math.ceil(data.total / LIMIT) - 1);
      await runSearch();
    }
  } catch (error) {
    if (seq === requestSeq) notify(String(error), "error");
  } finally {
    if (seq === requestSeq) {
      searched.value = true;
      loading.value = false;
    }
  }
}

function schedule(reset: boolean) {
  if (reset) page.value = 0;
  window.clearTimeout(timer);
  timer = window.setTimeout(() => {
    void runSearch();
  }, 220);
}

function goPage(next: number) {
  page.value = Math.min(Math.max(0, next), pageCount.value - 1);
  void runSearch();
}

function syncBasketFlags() {
  for (const item of results.value) {
    item.inBasket = basketIdSet.value.has(item.id);
  }
}

async function toggle(item: SearchItem) {
  try {
    await toggleBasket(item);
    syncBasketFlags();
  } catch (error) {
    notify(String(error), "error");
  }
}

async function toggleContextBasket() {
  if (!contextTarget.value) return;
  try {
    await toggleBasket(contextTarget.value);
    syncBasketFlags();
  } catch (error) {
    notify(String(error), "error");
  }
}

async function toggleAll() {
  try {
    if (allSelected.value) {
      const ids = pageItems.value.map((item) => item.id);
      await api.removeFromBasket(ids);
      await refreshBasket();
      syncBasketFlags();
      return;
    }
    const missing = pageItems.value.filter((item) => !basketIdSet.value.has(item.id));
    await api.addToBasket(missing.map((item) => item.id));
    await refreshBasket();
    syncBasketFlags();
  } catch (error) {
    notify(String(error), "error");
  }
}

function highlighted(text: string) {
  return splitHighlight(text, keyword.value.trim());
}

async function openContext(item: SearchItem) {
  contextTarget.value = item;
  contextItems.value = [];
  contextLoading.value = true;
  try {
    contextItems.value = await api.messageContext(item.id, 8);
  } catch (error) {
    notify(String(error), "error");
  } finally {
    contextLoading.value = false;
  }
}

function closeContext() {
  contextTarget.value = null;
  contextItems.value = [];
}

onMounted(async () => {
  try {
    await refreshBasket();
    const [groupList, personList] = await Promise.all([
      api.listGroups(true),
      api.searchPeople("", true),
    ]);
    groups.value = groupList;
    people.value = personList;
  } catch (error) {
    notify(String(error), "error");
  }
  await runSearch();
});

onBeforeUnmount(() => {
  window.clearTimeout(timer);
});
</script>

<template>
  <div class="search-page">
    <aside class="search-filters">
      <div class="search-keyword">
        <input
          v-model="keyword"
          class="search-input"
          placeholder="搜索聊天内容"
          @input="schedule(true)"
          @keyup.enter="runSearch"
        />
      </div>
      <div class="segmented compact">
        <button
          :class="{ active: keywordMode === 'fuzzy' }"
          @click="keywordMode = 'fuzzy'; schedule(true)"
        >
          模糊
        </button>
        <button
          :class="{ active: keywordMode === 'exact' }"
          @click="keywordMode = 'exact'; schedule(true)"
        >
          精确
        </button>
      </div>

      <div class="chip-row">
        <button class="chip" :class="{ active: isQuickRange(0) }" @click="quickRange(0)">
          今天
        </button>
        <button class="chip" :class="{ active: isQuickRange(7) }" @click="quickRange(7)">
          近7天
        </button>
        <button class="chip" :class="{ active: isQuickRange(30) }" @click="quickRange(30)">
          近30天
        </button>
      </div>
      <div class="date-row">
        <input v-model="startDate" class="field-input" type="date" @change="schedule(true)" />
        <input v-model="endDate" class="field-input" type="date" @change="schedule(true)" />
      </div>

      <div class="suggest-wrap">
        <input
          :value="senderInput"
          class="search-input"
          placeholder="发送人"
          @input="updateSender(($event.target as HTMLInputElement).value)"
          @focus="senderOpen = true"
          @blur="senderOpen = false"
        />
        <div v-if="senderOpen && senderMatches.length" class="suggest-list">
          <button
            v-for="person in senderMatches"
            :key="person.senderId"
            class="suggest-item"
            @mousedown.prevent="pickSender(person)"
          >
            <span>{{ person.senderName }}</span>
            <span class="muted">{{ person.messageCount }} 条</span>
          </button>
        </div>
      </div>

      <input
        v-model="mention"
        class="search-input"
        placeholder="@人"
        @input="schedule(true)"
      />

      <div class="chip-row">
        <button
          v-for="type in TYPE_ORDER"
          :key="type"
          class="chip"
          :class="{ active: types.includes(type) }"
          @click="toggleType(type)"
        >
          {{ TYPE_LABEL[type] }}
        </button>
      </div>

      <input
        v-model="groupQuery"
        class="search-input"
        placeholder="搜索群名"
      />
      <div class="search-group-list">
        <button
          v-for="group in filteredGroups"
          :key="group.id"
          class="chip"
          :class="{ active: groupIds.includes(group.id) }"
          @click="toggleGroup(group.id)"
        >
          {{ group.name }}
        </button>
        <span v-if="filteredGroups.length === 0" class="muted">没有匹配的群</span>
      </div>

      <div class="chip-row">
        <button class="chip" :class="{ active: inBasketOnly }" @click="toggleBasketOnly">
          已选消息
        </button>
        <button class="chip" :class="{ active: inRecordOnly }" @click="toggleRecordOnly">
          已录入
        </button>
        <select v-model="sort" class="field-input sort-select" @change="schedule(true)">
          <option value="desc">时间从新到旧</option>
          <option value="asc">时间从旧到新</option>
        </select>
      </div>
    </aside>

    <section class="search-results">
      <div class="toolbar">
        <label class="check-label">
          <input type="checkbox" :checked="allSelected" @change="toggleAll" />
          全选本页
        </label>
        <span class="sel-info">共 {{ total }} 条</span>
        <div class="grow"></div>
        <button class="btn small" @click="basket.open = true">
          已选消息 {{ basket.items.length }}
        </button>
      </div>

      <div class="search-list">
        <div v-if="loading && results.length === 0" class="empty">加载中</div>
        <div v-else-if="searched && results.length === 0" class="empty">
          <div class="empty-title">没有匹配的消息</div>
        </div>
        <div
          v-for="item in results"
          :key="item.id"
          class="message-item"
          :class="{ selected: item.inBasket }"
          @click="openContext(item)"
        >
          <input
            class="msg-check"
            type="checkbox"
            :checked="item.inBasket"
            @click.stop
            @change="toggle(item)"
          />
          <div class="msg-main">
            <div class="msg-top">
              <span class="sender">
                {{ item.senderName }}
                <span class="group-tag">{{ item.groupName }}</span>
              </span>
              <span class="sender-card">
                <span v-if="item.inRecord" class="state-tag">已录入</span>
                <span class="type-badge">{{ TYPE_LABEL[item.type] }}</span>
                <span class="msg-time">{{ formatTime(item.createdAt) }}</span>
              </span>
            </div>
            <div v-if="item.quotedContent" class="quote-block">
              引用 {{ item.quotedSender }}：{{ item.quotedContent }}
            </div>
            <div v-if="item.type === 'image'" class="image-chip">
              图片 · {{ item.fileName }}
            </div>
            <div v-else-if="item.type === 'file'" class="file-chip">
              文件 · {{ item.fileName }}
              <span class="size">{{ item.fileSize }}</span>
            </div>
            <div v-if="item.content" class="msg-content">
              <template v-for="(part, index) in highlighted(item.content)" :key="index">
                <mark v-if="part.mark">{{ part.text }}</mark>
                <template v-else>{{ part.text }}</template>
              </template>
            </div>
          </div>
        </div>
      </div>

      <div class="message-foot">
        <span>第 {{ page + 1 }} / {{ pageCount }} 页</span>
        <div class="pager">
          <button class="text-btn" :disabled="page === 0" @click="goPage(page - 1)">
            上一页
          </button>
          <button
            class="text-btn"
            :disabled="page >= pageCount - 1"
            @click="goPage(page + 1)"
          >
            下一页
          </button>
        </div>
      </div>
    </section>

    <BasketDrawer @changed="syncBasketFlags" />

    <div v-if="contextTarget" class="modal-backdrop" @click.self="closeContext">
      <div class="modal wide">
        <div class="modal-head">
          <div class="modal-title">{{ contextTarget.groupName }}</div>
          <button class="text-btn" @click="closeContext">关闭</button>
        </div>
        <div class="modal-body context-body">
          <div v-if="contextLoading" class="extract-loading">加载中…</div>
          <div
            v-for="message in contextItems"
            :key="message.id"
            class="context-item"
            :class="{ hit: message.id === contextTarget.id }"
          >
            <div class="context-meta">
              <span class="sender">{{ message.senderName }}</span>
              <span>{{ formatTime(message.createdAt) }}</span>
            </div>
            <div v-if="message.quotedContent" class="quote-block">
              引用 {{ message.quotedSender }}：{{ message.quotedContent }}
            </div>
            <div v-if="message.type === 'image'" class="image-chip">
              图片 · {{ message.fileName }}
            </div>
            <div v-else-if="message.type === 'file'" class="file-chip">
              文件 · {{ message.fileName }}
              <span class="size">{{ message.fileSize }}</span>
            </div>
            <div v-if="message.content" class="context-text">
              <template v-if="message.id === contextTarget.id">
                <template
                  v-for="(part, index) in splitHighlight(message.content, keyword.trim())"
                  :key="index"
                >
                  <mark v-if="part.mark">{{ part.text }}</mark>
                  <template v-else>{{ part.text }}</template>
                </template>
              </template>
              <template v-else>{{ message.content }}</template>
            </div>
          </div>
        </div>
        <div class="modal-foot">
          <button class="btn" @click="toggleContextBasket">
            {{ contextIsInBasket ? "移出已选" : "加入已选" }}
          </button>
          <button class="btn primary" @click="basket.open = true; closeContext()">
            打开已选消息
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

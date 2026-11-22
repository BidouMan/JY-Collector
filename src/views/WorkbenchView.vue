<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { api } from "../api";
import BasketDrawer from "../components/BasketDrawer.vue";
import ImageLightbox from "../components/ImageLightbox.vue";
import MessageEmoticon from "../components/MessageEmoticon.vue";
import MessageImage from "../components/MessageImage.vue";
import ScopeDialog from "../components/ScopeDialog.vue";
import { collector } from "../services/collector";
import {
  avatarInitial,
  avatarUrl,
  ensureAvatars,
  ensureAvatarsFor,
  loadAvatars,
} from "../stores/avatars";
import { clearImageCache } from "../stores/images";
import {
  basket,
  basketIdSet,
  clearBasketNow,
  refreshBasket,
  removeFromBasket,
  toggleBasket,
} from "../stores/basket";
import { notify } from "../toast";
import type { Group, Message, Person } from "../types";
import { formatTime, splitHighlight, TYPE_LABEL } from "../utils";
import { replaceWechatEmoji } from "../wechatEmoji";

const groups = ref<Group[]>([]);
const mode = ref<"group" | "person">("group");
const searchText = ref("");
const people = ref<Person[]>([]);
const peopleLoading = ref(false);
const activeGroupId = ref("");
const activePerson = ref<Person | null>(null);
const messages = ref<Message[]>([]);
const messageLoading = ref(false);
const messageListRef = ref<HTMLElement | null>(null);
const previewMd5 = ref("");
const keyword = ref("");
const groupsOpen = ref(false);
const scopeOpen = ref(false);
const refreshing = ref(false);
const dataUpdated = ref(false);
/** 数据源不可用时的说明（例如 Windows 版暂不支持读取本机微信）。 */
const sourceNotice = ref("");
/** 口令/密钥失效：给出“重新授权”入口，不弹全屏向导打断采集。 */
const needsAuth = ref(false);
let searchTimer: number | undefined;
let updateTimer: number | undefined;

const activeGroup = computed(() =>
  groups.value.find((group) => group.id === activeGroupId.value),
);

/** 会话按最后活跃时间排序，刷新后自动回到“最新消息在最上面”。 */
function lastActiveSeconds(value: string | null) {
  if (!value) return 0;
  const ms = Date.parse(value.replace(" ", "T"));
  return Number.isNaN(ms) ? 0 : ms / 1000;
}

const filteredGroups = computed(() => {
  const query = searchText.value.trim().toLowerCase();
  const list = query
    ? groups.value.filter((group) => group.name.toLowerCase().includes(query))
    : groups.value;
  return [...list].sort((a, b) => lastActiveSeconds(b.lastAt) - lastActiveSeconds(a.lastAt));
});

const filteredPeople = computed(() => {
  const query = searchText.value.trim().toLowerCase();
  if (!query) return people.value;
  return people.value.filter(
    (person) =>
      person.senderName.toLowerCase().includes(query) ||
      person.groupNames.toLowerCase().includes(query),
  );
});

const followedPeople = ref<Person[]>([]);

/** 关注页：有搜索词时展示全量搜索结果用于添加关注，否则只展示已关注的人。 */
const personList = computed(() =>
  searchText.value.trim() ? filteredPeople.value : followedPeople.value,
);

const conversationTitle = computed(() =>
  mode.value === "person"
    ? activePerson.value?.senderName ?? "成员"
    : activeGroup.value?.name ?? "消息池",
);

const conversationMeta = computed(() => {
  if (mode.value === "person") {
    if (!activePerson.value) return "";
    return `${activePerson.value.groupCount} 个群 · ${activePerson.value.messageCount} 条消息`;
  }
  if (!activeGroup.value) return "";
  const head =
    activeGroup.value.kind === "private" ? "单聊" : `${activeGroup.value.memberCount} 人`;
  return `${head} · ${activeGroup.value.messageCount} 条消息 · ${activeGroup.value.recordCount} 条记录`;
});

const filteredMessages = computed(() => {
  const query = keyword.value.trim().toLowerCase();
  if (!query) return messages.value;
  return messages.value.filter(
    (message) =>
      message.content.toLowerCase().includes(query) ||
      message.senderName.toLowerCase().includes(query) ||
      message.groupName.toLowerCase().includes(query) ||
      (message.fileName ?? "").toLowerCase().includes(query),
  );
});

const basketCount = computed(() => basket.items.length);

const conversationAvatarId = computed(() =>
  mode.value === "person" ? activePerson.value?.senderId ?? "" : activeGroupId.value,
);

const conversationName = computed(() =>
  mode.value === "person"
    ? activePerson.value?.senderName ?? ""
    : activeGroup.value?.name ?? "",
);

const allSelected = computed(
  () =>
    filteredMessages.value.length > 0 &&
    filteredMessages.value.every((message) => basketIdSet.value.has(message.id)),
);

async function loadGroups() {
  try {
    groups.value = await collector.listGroups();
  } catch (error) {
    notify(String(error), "error");
  }
  ensureAvatarsFor(groups.value.map((group) => group.id));
}

async function loadMessages() {
  messageLoading.value = true;
  try {
    if (mode.value === "person" && activePerson.value) {
      messages.value = await collector.listPersonMessages(activePerson.value.senderId);
    } else if (mode.value === "group" && activeGroupId.value) {
      messages.value = await collector.listMessages(activeGroupId.value);
    } else {
      messages.value = [];
    }
  } catch (error) {
    notify(String(error), "error");
    messages.value = [];
  } finally {
    messageLoading.value = false;
  }
  ensureAvatarsFor(messages.value.map((message) => message.senderId));
  await scrollToLatest();
}

/** 切换会话后默认定位到最新一条消息（列表底部）。 */
async function scrollToLatest() {
  await nextTick();
  const list = messageListRef.value;
  if (list) list.scrollTop = list.scrollHeight;
}

async function loadPeople(query: string) {
  peopleLoading.value = true;
  try {
    people.value = await collector.searchPeople(query.trim());
  } catch (error) {
    notify(String(error), "error");
    people.value = [];
  } finally {
    peopleLoading.value = false;
  }
  ensureAvatarsFor(people.value.map((person) => person.senderId));
}

async function loadFollowed() {
  try {
    followedPeople.value = await api.listFollowed("");
  } catch (error) {
    notify(String(error), "error");
    followedPeople.value = [];
  }
  ensureAvatarsFor(followedPeople.value.map((person) => person.senderId));
}

/** 关注或取消关注；搜索结果与关注列表同步刷新。 */
async function setFollowed(person: Person, followed: boolean) {
  try {
    await api.toggleFollow(person.senderId, person.senderName, followed);
    person.isFollowed = followed;
    await loadFollowed();
  } catch (error) {
    notify(String(error), "error");
  }
}

async function selectGroup(group: Group) {
  mode.value = "group";
  activeGroupId.value = group.id;
  activePerson.value = null;
  keyword.value = "";
  groupsOpen.value = false;
  await loadMessages();
}

function selectPerson(person: Person) {
  activePerson.value = person;
  keyword.value = "";
  groupsOpen.value = false;
  void loadMessages();
}

function switchMode(next: "group" | "person") {
  if (mode.value === next) return;
  mode.value = next;
  searchText.value = "";
  keyword.value = "";
  activePerson.value = null;
  messages.value = [];
  if (next === "person") {
    people.value = [];
    void loadFollowed();
  } else if (groups.value[0]) {
    void selectGroup(groups.value[0]);
  }
}

function isSelected(id: string) {
  return basketIdSet.value.has(id);
}

async function toggle(message: Message) {
  try {
    await toggleBasket(message);
  } catch (error) {
    notify(String(error), "error");
  }
}

async function toggleAll() {
  try {
    const filtered = filteredMessages.value;
    if (allSelected.value) {
      await removeFromBasket(filtered.map((message) => message.id));
    } else {
      const missing = filtered.filter((message) => !basketIdSet.value.has(message.id));
      await api.addToBasket(missing.map((message) => message.id));
      await refreshBasket();
    }
  } catch (error) {
    notify(String(error), "error");
  }
}

async function clearSelection() {
  try {
    await clearBasketNow();
  } catch (error) {
    notify(String(error), "error");
  }
}

function highlighted(text: string) {
  return splitHighlight(text, keyword.value.trim());
}

watch(searchText, (value) => {
  if (mode.value !== "person") return;
  window.clearTimeout(searchTimer);
  const query = value.trim();
  if (!query) {
    people.value = [];
    return;
  }
  searchTimer = window.setTimeout(() => {
    void loadPeople(query);
  }, 180);
});

/** 只 stat 微信消息库，不触发解密；失败时静默。 */
async function checkUpdated() {
  if (refreshing.value) return;
  try {
    dataUpdated.value = await api.sourceUpdated();
  } catch {
    dataUpdated.value = false;
  }
}

/** 同步失败或向导完成后刷新数据源状态，决定是否显示重新授权入口。 */
async function refreshStatus() {
  try {
    const status = await api.sourceStatus();
    needsAuth.value = status.needsAuth;
    if (status.ready) sourceNotice.value = "";
  } catch {
    needsAuth.value = false;
  }
}

function openSetup() {
  window.dispatchEvent(
    new CustomEvent("jy:open-setup", { detail: { mode: "reauth" } }),
  );
}

async function refresh() {
  if (refreshing.value) return;  refreshing.value = true;
  try {
    const summary = await api.refreshSource();
    sourceNotice.value = "";
    needsAuth.value = false;
    clearImageCache();
    await loadAvatars(true);
    await loadGroups();
    if (mode.value === "group") {
      const current = groups.value.find((group) => group.id === activeGroupId.value);
      if (current) {
        await loadMessages();
      } else if (groups.value[0]) {
        await selectGroup(groups.value[0]);
      } else {
        activeGroupId.value = "";
        messages.value = [];
      }
    } else if (activePerson.value) {
      await loadMessages();
      await loadFollowed();
    }
    await refreshBasket();
    notify(`已更新 ${summary.added} 条消息`);
    dataUpdated.value = false;
  } catch (error) {
    notify(String(error), "error");
    await refreshStatus();
  } finally {
    refreshing.value = false;
  }
}

/** 设置里改过采集范围后，重新拉取可见的群/成员。 */
async function onScopeChanged() {
  await refreshStatus();
  await loadGroups();
  if (mode.value === "person") {
    await loadFollowed();
    if (searchText.value.trim()) await loadPeople(searchText.value);
    return;
  }
  if (groups.value.some((group) => group.id === activeGroupId.value)) {
    await loadMessages();
    return;
  }
  if (groups.value[0]) {
    await selectGroup(groups.value[0]);
  } else {
    activeGroupId.value = "";
    messages.value = [];
  }
}

onMounted(async () => {
  window.addEventListener("jy:scope-changed", onScopeChanged);
  updateTimer = window.setInterval(() => void checkUpdated(), 5000);
  try {
    await Promise.all([refreshBasket(), ensureAvatars()]);
    const [groupList, status] = await Promise.all([collector.listGroups(), api.sourceStatus()]);
    groups.value = groupList;
    needsAuth.value = status.needsAuth;
    if (!status.ready && status.lastError) {
      // 没有会话时不弹报错，直接在左侧列表显示一条状态说明。
      sourceNotice.value = status.lastError;
      if (groups.value.length > 0) notify(status.lastError, "error");
      if (groups.value[0]) await selectGroup(groups.value[0]);
      return;
    }
    if (groups.value.length === 0) {
      await refresh();
      return;
    }
    if (groups.value[0]) await selectGroup(groups.value[0]);
  } catch (error) {
    notify(String(error), "error");
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("jy:scope-changed", onScopeChanged);
  window.clearTimeout(searchTimer);
  window.clearInterval(updateTimer);
});
</script>

<template>
  <div class="workbench" :class="{ 'groups-open': groupsOpen }">
    <div v-if="groupsOpen" class="basket-backdrop" @click="groupsOpen = false"></div>

    <aside class="group-pane">
      <div class="pane-head">
        <div class="pane-head-row">
          <div class="segmented">
            <button :class="{ active: mode === 'group' }" @click="switchMode('group')">
              会话
            </button>
            <button :class="{ active: mode === 'person' }" @click="switchMode('person')">
              关注
            </button>
          </div>
          <div class="head-actions">
            <span class="refresh-slot">
              <span v-if="dataUpdated" class="update-dot"></span>
              <button class="text-btn" :disabled="refreshing" @click="refresh">
                {{ refreshing ? "刷新中…" : "刷新" }}
              </button>
            </span>
            <button class="text-btn" @click="scopeOpen = true; groupsOpen = false">管理</button>
            <button class="text-btn group-close" @click="groupsOpen = false">关闭</button>
          </div>
        </div>
        <input
          v-model="searchText"
          class="search-input"
          :placeholder="mode === 'group' ? '搜索会话或联系人' : '搜索成员并关注'"
        />
      </div>

      <div v-if="mode === 'group'" class="list-scroll">
        <button
          v-for="group in filteredGroups"
          :key="group.id"
          class="group-item"
          :class="{ active: group.id === activeGroupId }"
          @click="selectGroup(group)"
        >
          <span class="list-avatar">
            <img v-if="avatarUrl(group.id)" :src="avatarUrl(group.id)" alt="" />
            <span v-else>{{ avatarInitial(group.name) }}</span>
          </span>
          <span class="item-body">
            <div class="item-name">
              <span class="item-time">{{ formatTime(group.lastAt) }}</span>
              {{ group.name }}
            </div>
            <div class="item-meta">
              {{ group.kind === "private" ? "单聊" : `${group.memberCount} 人` }} ·
              {{ group.messageCount }} 条
              <template v-if="group.recordCount"> · {{ group.recordCount }} 条记录</template>
            </div>
          </span>
        </button>
        <div v-if="filteredGroups.length === 0" class="empty">
          {{ sourceNotice || "没有匹配的会话" }}
        </div>
      </div>

      <div v-else class="list-scroll">
        <div
          v-for="person in personList"
          :key="person.senderId"
          class="group-item follow-row"
          :class="{ active: person.senderId === activePerson?.senderId }"
          @click="selectPerson(person)"
        >
          <span class="list-avatar">
            <img v-if="avatarUrl(person.senderId)" :src="avatarUrl(person.senderId)" alt="" />
            <span v-else>{{ avatarInitial(person.senderName) }}</span>
          </span>
          <span class="item-body">
            <div class="item-name">{{ person.senderName }}</div>
            <div class="item-meta">
              <template v-if="person.messageCount">
                {{ person.groupCount }} 个会话 · {{ person.messageCount }} 条 ·
                {{ person.groupNames }}
              </template>
              <template v-else>暂无消息</template>
            </div>
          </span>
          <button
            class="text-btn follow-toggle"
            @click.stop="setFollowed(person, !person.isFollowed)"
          >
            {{ person.isFollowed ? "取消关注" : "关注" }}
          </button>
        </div>
        <div v-if="peopleLoading" class="empty">加载中</div>
        <div v-else-if="personList.length === 0" class="empty">
          <div v-if="searchText.trim()">没有匹配的成员，搜索受采集范围限制</div>
          <template v-else>
            <div class="empty-title">还没有关注的人</div>
            <div>在上方搜索成员，点击“关注”即可加入这里</div>
          </template>
        </div>
      </div>
    </aside>

    <section class="message-pane">
      <div class="conversation-head">
        <div class="conversation-identity">
          <span class="list-avatar head-avatar">
            <img
              v-if="avatarUrl(conversationAvatarId)"
              :src="avatarUrl(conversationAvatarId)"
              alt=""
            />
            <span v-else>{{ avatarInitial(conversationName) }}</span>
          </span>
          <div class="conversation-text">
            <div class="conversation-title">{{ conversationTitle }}</div>
            <div class="conversation-meta">{{ conversationMeta }}</div>
          </div>
        </div>
        <div class="head-actions">
          <button class="btn small narrow-only" @click="groupsOpen = true">会话</button>
          <button class="btn small" @click="basket.open = true">
            已选消息 {{ basketCount }}
          </button>
        </div>
      </div>

      <div v-if="needsAuth" class="reauth-strip">
        <span>微信数据不可用，暂时无法同步</span>
        <button class="text-btn" @click="openSetup">重新连接</button>
      </div>

      <div class="toolbar">
        <div class="grow">
          <input v-model="keyword" class="search-input" placeholder="筛选消息内容或发送人" />
        </div>
        <label class="check-label">
          <input type="checkbox" :checked="allSelected" @change="toggleAll" />
          全选
        </label>
        <span class="sel-info">已选消息 {{ basketCount }} 条</span>
        <button class="text-btn" @click="clearSelection">清空</button>
      </div>

      <div ref="messageListRef" class="message-list">
        <div v-if="messageLoading" class="empty">加载中</div>
        <div v-else-if="filteredMessages.length === 0" class="empty">
          <div class="empty-title">
            {{ mode === "person" && !activePerson ? "选择关注的人查看发言" : "没有匹配的消息" }}
          </div>
        </div>
        <template v-else>
          <div
            v-for="message in filteredMessages"
            :key="message.id"
            class="message-item"
            :class="{ selected: isSelected(message.id) }"
            @click="toggle(message)"
          >
            <input
              class="msg-check"
              type="checkbox"
              :checked="isSelected(message.id)"
              @click.stop
              @change="toggle(message)"
            />
            <span class="msg-avatar">
              <img
                v-if="avatarUrl(message.senderId)"
                :src="avatarUrl(message.senderId)"
                alt=""
              />
              <span v-else>{{ avatarInitial(message.senderName) }}</span>
            </span>
            <div class="msg-main">
              <div class="msg-top">
                <span class="sender">
                  {{ message.senderName }}
                  <span v-if="mode === 'person'" class="group-tag">
                    {{ message.groupName }}
                  </span>
                </span>
                <span class="sender-card">
                  <span v-if="message.type !== 'text'" class="type-badge">
                    {{ TYPE_LABEL[message.type] }}
                  </span>
                  <span class="msg-time">{{ formatTime(message.createdAt) }}</span>
                </span>
              </div>
              <div v-if="message.quotedContent" class="quote-block">
                引用 {{ message.quotedSender }}：{{
                  replaceWechatEmoji(message.quotedContent)
                }}
              </div>
              <MessageEmoticon
                v-if="message.type === 'emoticon' && message.emoticonMd5"
                :message-id="message.id"
                :md5="message.emoticonMd5"
              />
              <MessageImage
                v-if="message.type === 'image' && message.imageMd5"
                :md5="message.imageMd5"
                @preview="previewMd5 = $event"
              />
              <div v-else-if="message.type === 'image'" class="image-chip">
                图片 · {{ message.fileName }}
              </div>
              <div v-else-if="message.type === 'file'" class="file-chip">
                文件 · {{ message.fileName }}
                <span class="size">{{ message.fileSize }}</span>
              </div>
              <div
                v-if="message.content && !(message.type === 'emoticon' && message.emoticonMd5)"
                class="msg-content"
              >
                <template v-for="(part, index) in highlighted(message.content)" :key="index">
                  <mark v-if="part.mark">{{ part.text }}</mark>
                  <template v-else>{{ part.text }}</template>
                </template>
              </div>
            </div>
          </div>
        </template>
      </div>

      <div class="message-foot">
        <span>{{ filteredMessages.length }} 条消息</span>
        <span>{{ messages.length }} 条原始记录</span>
      </div>
    </section>

    <BasketDrawer @changed="loadGroups" />
    <ScopeDialog v-if="scopeOpen" @close="scopeOpen = false" />
    <ImageLightbox v-if="previewMd5" :md5="previewMd5" @close="previewMd5 = ''" />
  </div>
</template>

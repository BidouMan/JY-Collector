<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api } from "../api";
import type { SetupStatus, SyncSummary } from "../api";
import type { CaptureScope, Group } from "../types";

const props = withDefaults(defineProps<{ mode?: "first" | "reauth" }>(), {
  mode: "first",
});

const emit = defineEmits<{ done: []; close: [] }>();

const steps = ["检测", "授权", "同步", "选择会话"];

const step = ref(1);
const status = ref<SetupStatus | null>(null);
const checking = ref(false);
const checkError = ref("");
const authAttempted = ref(false);
const syncSummary = ref<SyncSummary | null>(null);
const syncing = ref(false);
const syncError = ref("");
const elapsed = ref(0);
const scope = ref<CaptureScope | null>(null);
const groups = ref<Group[]>([]);
const groupIds = ref<string[]>([]);
const scopeLoading = ref(false);
const scopeError = ref("");
const query = ref("");
const saving = ref(false);
let timer: number | undefined;

const filteredGroups = computed(() => {
  const keyword = query.value.trim().toLowerCase();
  if (!keyword) return groups.value;
  return groups.value.filter((group) => group.name.toLowerCase().includes(keyword));
});

const canFinish = computed(
  () => groups.value.length === 0 || groupIds.value.length > 0,
);

const footerNote = computed(() => {
  if (step.value === 1) return "只检测本机微信数据与读取授权，不改动任何微信文件";
  if (step.value === 2) return "授权由本机钥匙串提供，口令不会显示在界面或写入仓库";
  if (step.value === 3) return "同步只读取微信数据，并写入本应用自己的本地库";
  return "勾选的会话决定采集页、更新提示与查找范围";
});

function humanError(error: unknown) {
  const text = String(error ?? "");
  if (text.includes("python")) return "本机缺少采集组件，暂时无法同步";
  if (text.includes("采集脚本缺失")) return "采集组件缺失，请重新安装应用";
  return text || "操作失败，请重试";
}

async function detect() {
  if (checking.value) return;
  checking.value = true;
  checkError.value = "";
  try {
    status.value = await api.setupStatus();
  } catch (error) {
    checkError.value = humanError(error);
  } finally {
    checking.value = false;
  }
}

async function loadScope() {
  scopeLoading.value = true;
  scopeError.value = "";
  try {
    const result = await api.getCaptureScope();
    scope.value = result;
    groups.value = result.groups;
    groupIds.value = result.configured
      ? result.selectedGroups.filter((id) =>
          result.groups.some((group) => group.id === id),
        )
      : result.groups.map((group) => group.id);
  } catch (error) {
    scopeError.value = humanError(error);
  } finally {
    scopeLoading.value = false;
  }
}

function isChecked(id: string) {
  return groupIds.value.includes(id);
}

function toggle(id: string) {
  groupIds.value = isChecked(id)
    ? groupIds.value.filter((item) => item !== id)
    : [...groupIds.value, id];
}

function selectAll() {
  const visible = filteredGroups.value.map((group) => group.id);
  groupIds.value = Array.from(new Set([...groupIds.value, ...visible]));
}

function clearAll() {
  const visible = new Set(filteredGroups.value.map((group) => group.id));
  groupIds.value = groupIds.value.filter((id) => !visible.has(id));
}

function startTimer() {
  elapsed.value = 0;
  window.clearInterval(timer);
  timer = window.setInterval(() => {
    elapsed.value += 1;
  }, 1000);
}

function stopTimer() {
  window.clearInterval(timer);
  timer = undefined;
}

async function runSync() {
  if (syncing.value) return;
  syncing.value = true;
  syncError.value = "";
  syncSummary.value = null;
  startTimer();
  try {
    syncSummary.value = await api.refreshSource();
  } catch (error) {
    syncError.value = humanError(error);
  } finally {
    stopTimer();
    syncing.value = false;
  }
}

/** 重新读钥匙串并校验口令；不做任何伪造的成功状态。 */
async function authorize() {
  authAttempted.value = true;
  await detect();
}

async function finish() {
  if (saving.value) return;
  saving.value = true;
  try {
    if (groups.value.length > 0) {
      const people = scope.value?.configured
        ? scope.value.selectedPeople
        : (scope.value?.people ?? []).map((person) => person.senderId);
      await api.saveCaptureScope(groupIds.value, people);
      window.dispatchEvent(new Event("jy:scope-changed"));
    }
    await api.completeSetup();
    emit("done");
  } catch (error) {
    scopeError.value = humanError(error);
  } finally {
    saving.value = false;
  }
}

/** 没有数据源（Windows 或本机无微信）时也允许走完，保证主界面能正常打开。 */
async function finishWithoutSource() {
  if (saving.value) return;
  saving.value = true;
  try {
    await api.completeSetup();
    emit("done");
  } catch (error) {
    checkError.value = humanError(error);
  } finally {
    saving.value = false;
  }
}

function next() {
  if (step.value === 1) {
    step.value = 2;
    return;
  }
  if (step.value === 2) {
    step.value = 3;
    return;
  }
  if (step.value === 3) {
    step.value = 4;
    void loadScope();
  }
}

function back() {
  if (step.value > 1) step.value -= 1;
}

onMounted(() => {
  void detect();
});

onBeforeUnmount(stopTimer);
</script>

<template>
  <div class="setup-backdrop">
    <div class="setup-panel">
      <header class="setup-head">
        <div>
          <div class="setup-title">{{ props.mode === "reauth" ? "重新授权" : "开始使用" }}</div>
          <div class="setup-sub">
            {{ props.mode === "reauth" ? "重新连接本机微信数据" : "按顺序完成四步即可开始采集" }}
          </div>
        </div>
        <button class="text-btn" @click="emit('close')">
          {{ props.mode === "reauth" ? "关闭" : "稍后设置" }}
        </button>
      </header>

      <ol class="setup-steps">
        <li
          v-for="(label, index) in steps"
          :key="label"
          :class="{ active: step === index + 1, done: step > index + 1 }"
        >
          <span class="setup-index">{{ index + 1 }}</span>
          <span>{{ label }}</span>
        </li>
      </ol>

      <div class="setup-body">
        <!-- 1 检测 -->
        <template v-if="step === 1">
          <div class="setup-status-list">
            <div class="setup-status-row">
              <span class="setup-status-label">微信数据</span>
              <span class="setup-status-value">
                <template v-if="status">
                  {{ status.dataFound ? `已找到（${status.accountCount} 个账号）` : "未找到" }}
                </template>
                <template v-else>{{ checking ? "检测中…" : "待检测" }}</template>
              </span>
            </div>
            <div class="setup-status-row">
              <span class="setup-status-label">读取授权</span>
              <span class="setup-status-value">
                <template v-if="status">
                  <template v-if="!status.supported">不需要</template>
                  <template v-else-if="status.authorized">已授权</template>
                  <template v-else-if="status.keyPresent">已失效，需要重新授权</template>
                  <template v-else>未授权</template>
                </template>
                <template v-else>—</template>
              </span>
            </div>
          </div>
          <p class="setup-message">
            {{ checkError || status?.message || "正在检测本机微信数据…" }}
          </p>
        </template>

        <!-- 2 授权 -->
        <template v-else-if="step === 2">
          <div class="setup-status-list">
            <div class="setup-status-row">
              <span class="setup-status-label">当前状态</span>
              <span class="setup-status-value">
                <template v-if="status && !status.supported">暂不支持</template>
                <template v-else-if="status?.authorized">已授权</template>
                <template v-else>未授权</template>
              </span>
            </div>
          </div>
          <p class="setup-message">
            <template v-if="status && !status.supported">
              Windows 版暂时无法读取本机微信数据，可以继续完成设置并使用其他功能。
            </template>
            <template v-else-if="status?.authorized">
              本机已有可用授权，可以直接进入同步。
            </template>
            <template v-else>
              这台电脑还没有可用的微信读取授权。完成本机授权后，点下方按钮重新检测。
            </template>
          </p>
          <p v-if="checkError" class="setup-error">{{ checkError }}</p>
          <p v-else-if="authAttempted && !status?.authorized" class="setup-message">
            仍未检测到可用授权。完成本机授权后，再点一次“重新授权”。
          </p>
          <button
            v-if="status?.supported && !status?.authorized"
            class="btn small"
            :disabled="checking"
            @click="authorize"
          >
            {{ checking ? "检测中…" : "重新授权" }}
          </button>
        </template>

        <!-- 3 同步 -->
        <template v-else-if="step === 3">
          <template v-if="!status?.supported">
            <p class="setup-message">
              当前系统不支持读取本机微信数据，同步不可用。可以继续完成设置。
            </p>
          </template>
          <template v-else-if="!status?.authorized">
            <p class="setup-message">还没有可用授权，返回上一步完成授权后再同步。</p>
          </template>
          <template v-else-if="syncing">
            <p class="setup-message">正在读取微信聊天记录并写入本地库，请稍候…</p>
            <div class="setup-progress"><span></span></div>
            <div class="setup-meta">已用时 {{ elapsed }} 秒</div>
          </template>
          <template v-else-if="syncSummary">
            <div class="setup-done">
              <span class="setup-done-mark">✓</span>
              <div>
                <div class="setup-done-title">同步完成</div>
                <div class="setup-meta">
                  共 {{ syncSummary.groups }} 个会话
                  <template v-if="syncSummary.added > 0">
                    ，本次更新 {{ syncSummary.added }} 条消息
                  </template>
                  <template v-else>，数据已是最新</template>
                </div>
              </div>
            </div>
          </template>
          <template v-else>
            <p class="setup-message">
              将读取本机微信聊天记录，写入本应用自己的本地库。会话较多时可能需要一会儿。
            </p>
          </template>
          <p v-if="syncError" class="setup-error">{{ syncError }}</p>
        </template>

        <!-- 4 选择会话 -->
        <template v-else>
          <div class="setup-scope-head">
            <input v-model="query" class="search-input" placeholder="搜索会话" />
            <button class="text-btn" @click="selectAll">全选</button>
            <button class="text-btn" @click="clearAll">清空</button>
          </div>
          <div class="setup-scope-list">
            <label v-for="group in filteredGroups" :key="group.id" class="scope-row">
              <input
                type="checkbox"
                :checked="isChecked(group.id)"
                @change="toggle(group.id)"
              />
              <span class="scope-name">{{ group.name }}</span>
              <span class="scope-meta">
                {{ group.kind === "private" ? "单聊" : `${group.memberCount} 人` }} ·
                {{ group.messageCount }} 条
              </span>
            </label>
            <div v-if="scopeLoading" class="empty">加载中</div>
            <div v-else-if="filteredGroups.length === 0" class="empty">
              {{ query.trim() ? "没有匹配的会话" : "同步结果里还没有会话" }}
            </div>
          </div>
          <div class="setup-meta">已选 {{ groupIds.length }} / {{ groups.length }} 个会话</div>
          <p v-if="scopeError" class="setup-error">{{ scopeError }}</p>
        </template>
      </div>

      <footer class="setup-foot">
        <div class="setup-note">{{ footerNote }}</div>
        <div class="setup-actions">
          <button v-if="step > 1" class="btn small" @click="back">上一步</button>

          <template v-if="step === 1">
            <button
              v-if="status && !status.supported"
              class="btn small primary"
              :disabled="saving"
              @click="finishWithoutSource"
            >
              完成设置
            </button>
            <template v-else>
              <button
                v-if="status && (status.state === 'no_data' || status.state === 'error')"
                class="btn small"
                :disabled="checking"
                @click="detect"
              >
                {{ checking ? "检测中…" : "重新检测" }}
              </button>
              <button
                class="btn small primary"
                :disabled="checking || !status || status.state === 'no_data' || status.state === 'error'"
                @click="next"
              >
                下一步
              </button>
            </template>
          </template>

          <template v-else-if="step === 2">
            <button
              v-if="!status?.supported"
              class="btn small primary"
              :disabled="saving"
              @click="finishWithoutSource"
            >
              完成设置
            </button>
            <button
              v-else
              class="btn small primary"
              :disabled="!status?.authorized"
              @click="next"
            >
              下一步
            </button>
          </template>

          <template v-else-if="step === 3">
            <button
              v-if="status?.supported && status?.authorized && !syncSummary"
              class="btn small primary"
              :disabled="syncing"
              @click="runSync"
            >
              {{ syncing ? "同步中…" : syncError ? "重试" : "开始同步" }}
            </button>
            <button v-else class="btn small primary" :disabled="!syncSummary" @click="next">
              下一步
            </button>
          </template>

          <template v-else>
            <button
              class="btn small primary"
              :disabled="saving || !canFinish || scopeLoading"
              @click="finish"
            >
              {{ saving ? "保存中…" : "完成设置" }}
            </button>
          </template>
        </div>
      </footer>
    </div>
  </div>
</template>

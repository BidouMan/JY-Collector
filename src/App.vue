<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { api } from "./api";
import SetupWizard from "./components/SetupWizard.vue";
import { notify, toasts } from "./toast";
import LedgerView from "./views/LedgerView.vue";
import RecordsView from "./views/RecordsView.vue";
import SearchView from "./views/SearchView.vue";
import SettingsView from "./views/SettingsView.vue";
import WorkbenchView from "./views/WorkbenchView.vue";

type View = "collect" | "search" | "records" | "ledger" | "settings";

const view = ref<View>("collect");
const setupOpen = ref(false);
const setupMode = ref<"first" | "reauth">("first");

function onOpenSetup(event: Event) {
  const detail = (event as CustomEvent<{ mode?: "first" | "reauth" }>).detail;
  setupMode.value = detail?.mode === "reauth" ? "reauth" : "first";
  setupOpen.value = true;
}

function onSetupDone() {
  setupOpen.value = false;
  notify("设置已完成");
}

onMounted(async () => {
  window.addEventListener("jy:open-setup", onOpenSetup);
  try {
    if (!(await api.setupCompleted())) {
      setupMode.value = "first";
      setupOpen.value = true;
    }
  } catch {
    // 读取标志失败时不打断主界面，用户仍可手动操作。
  }
});

onBeforeUnmount(() => window.removeEventListener("jy:open-setup", onOpenSetup));
</script>

<template>
  <div class="app">
    <header class="appbar">
      <nav class="appnav">
        <button :class="{ active: view === 'collect' }" @click="view = 'collect'">
          采集
        </button>
        <button :class="{ active: view === 'search' }" @click="view = 'search'">
          查找
        </button>
        <button :class="{ active: view === 'ledger' }" @click="view = 'ledger'">
          数据
        </button>
        <button :class="{ active: view === 'records' }" @click="view = 'records'">
          记录
        </button>
        <button :class="{ active: view === 'settings' }" @click="view = 'settings'">
          设置
        </button>
      </nav>
      <div class="brand">
        <span class="brand-title">JY Collector</span>
        <img class="brand-mark" src="/app-icon.png" alt="JY" />
      </div>
    </header>

    <main class="app-main">
      <WorkbenchView v-show="view === 'collect'" />
      <SearchView v-if="view === 'search'" />
      <RecordsView v-if="view === 'records'" />
      <LedgerView v-if="view === 'ledger'" />
      <SettingsView v-if="view === 'settings'" />
    </main>

    <SetupWizard
      v-if="setupOpen"
      :mode="setupMode"
      @done="onSetupDone"
      @close="setupOpen = false"
    />

    <div class="toast-host">
      <div
        v-for="toast in toasts"
        :key="toast.id"
        class="toast"
        :class="{ error: toast.kind === 'error' }"
      >
        {{ toast.text }}
      </div>
    </div>
  </div>
</template>

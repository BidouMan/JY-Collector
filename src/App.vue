<script setup lang="ts">
import { ref } from "vue";
import LedgerView from "./views/LedgerView.vue";
import RecordsView from "./views/RecordsView.vue";
import SearchView from "./views/SearchView.vue";
import SettingsView from "./views/SettingsView.vue";
import WorkbenchView from "./views/WorkbenchView.vue";
import { toasts } from "./toast";

type View = "collect" | "search" | "records" | "ledger" | "settings";

const view = ref<View>("collect");
</script>

<template>
  <div class="app">
    <header class="appbar">
      <div class="brand">
        <span class="brand-mark">JY</span>
        <span class="brand-title">群录</span>
      </div>
      <nav class="appnav">
        <button :class="{ active: view === 'collect' }" @click="view = 'collect'">
          采集
        </button>
        <button :class="{ active: view === 'search' }" @click="view = 'search'">
          查找
        </button>
        <button :class="{ active: view === 'records' }" @click="view = 'records'">
          记录表
        </button>
        <button :class="{ active: view === 'ledger' }" @click="view = 'ledger'">
          台账
        </button>
        <button :class="{ active: view === 'settings' }" @click="view = 'settings'">
          设置
        </button>
      </nav>
    </header>

    <main class="app-main">
      <WorkbenchView v-show="view === 'collect'" />
      <SearchView v-if="view === 'search'" />
      <RecordsView v-if="view === 'records'" />
      <LedgerView v-if="view === 'ledger'" />
      <SettingsView v-if="view === 'settings'" />
    </main>

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

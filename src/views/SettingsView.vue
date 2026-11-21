<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api } from "../api";
import ExportSettingsPanel from "../components/ExportSettingsPanel.vue";
import { notify } from "../toast";
import type { Settings } from "../types";

const settings = ref<Settings | null>(null);
const apiKey = ref("");
const modelName = ref("deepseek-chat");
const savingModel = ref(false);

const dataItems = [
  { name: "自动备份", desc: "定期备份记录表与导出配置" },
  { name: "缓存清理", desc: "清理解密临时文件与过期缓存" },
];

const aboutItems = [{ name: "检查更新", desc: "获取并安装新版本" }];

async function saveModel() {
  savingModel.value = true;
  try {
    await api.saveSetting("api_key", apiKey.value.trim());
    await api.saveSetting("model_name", modelName.value.trim() || "deepseek-chat");
    notify("模型配置已保存");
  } catch (error) {
    notify(String(error), "error");
  } finally {
    savingModel.value = false;
  }
}

async function openDataDir() {
  try {
    await api.openDataDir();
  } catch (error) {
    notify(String(error), "error");
  }
}

onMounted(async () => {
  try {
    const config = await api.getSettings();
    settings.value = config;
    apiKey.value = config.apiKey;
    modelName.value = config.modelName;
  } catch (error) {
    notify(String(error), "error");
  }
});
</script>

<template>
  <div class="settings">
    <div class="settings-inner">
      <div class="page-head">
        <div class="page-title">设置</div>
      </div>

      <section class="setting-card">
        <div class="setting-card-head">
          <div>
            <div class="setting-card-title">模型接口</div>
            <div class="setting-card-desc">用于消息字段抽取的模型服务</div>
          </div>
          <button class="btn small" :disabled="savingModel" @click="saveModel">
            {{ savingModel ? "保存中…" : "保存" }}
          </button>
        </div>
        <div class="setting-card-body">
          <label class="card-field">
            <span class="card-field-label">API Key</span>
            <input
              v-model="apiKey"
              class="field-input"
              type="password"
              placeholder="填写模型服务商提供的 Key"
            />
          </label>
          <label class="card-field">
            <span class="card-field-label">模型名称</span>
            <input v-model="modelName" class="field-input" placeholder="deepseek-chat" />
            <span class="card-hint">
              密钥仅保存在本机。当前抽取使用本地规则，接入模型后界面无需改动。
            </span>
          </label>
        </div>
      </section>

      <section class="setting-card">
        <div class="setting-card-head">
          <div>
            <div class="setting-card-title">导出设置</div>
            <div class="setting-card-desc">台账模板、目标目录与列定义</div>
          </div>
        </div>
        <div class="setting-card-body export-body">
          <ExportSettingsPanel />
        </div>
      </section>

      <section class="setting-card">
        <div class="setting-card-head">
          <div>
            <div class="setting-card-title">数据管理</div>
            <div class="setting-card-desc">本地存储位置与数据维护</div>
          </div>
          <button class="btn small" @click="openDataDir">打开目录</button>
        </div>
        <div class="setting-card-body">
          <div class="setting-row">
            <div class="setting-row-main">
              <div class="setting-row-title">数据目录</div>
              <div class="setting-row-desc">
                <span class="dir-path" :title="settings?.dataDir">{{
                  settings?.dataDir || "读取中…"
                }}</span>
              </div>
            </div>
          </div>
          <div v-for="item in dataItems" :key="item.name" class="setting-row">
            <div class="setting-row-main">
              <div class="setting-row-title">{{ item.name }}</div>
              <div class="setting-row-desc">{{ item.desc }}</div>
            </div>
            <span class="soon-tag">规划中</span>
          </div>
        </div>
      </section>

      <section class="setting-card">
        <div class="setting-card-head">
          <div>
            <div class="setting-card-title">关于与更新</div>
            <div class="setting-card-desc">版本信息</div>
          </div>
        </div>
        <div class="setting-card-body">
          <div class="setting-row">
            <div class="setting-row-main">
              <div class="setting-row-title">群录</div>
              <div class="setting-row-desc">版本 0.1.0 · 本机运行，数据只保存在本地</div>
            </div>
            <span class="soon-tag">当前版本</span>
          </div>
          <div v-for="item in aboutItems" :key="item.name" class="setting-row">
            <div class="setting-row-main">
              <div class="setting-row-title">{{ item.name }}</div>
              <div class="setting-row-desc">{{ item.desc }}</div>
            </div>
            <span class="soon-tag">规划中</span>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

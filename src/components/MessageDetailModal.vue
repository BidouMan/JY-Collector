<script setup lang="ts">
import { computed } from "vue";
import type { Message } from "../types";
import { formatTime, splitHighlight, TYPE_LABEL } from "../utils";

const props = defineProps<{
  message: Message;
  snippet?: string | null;
  title?: string;
}>();

const emit = defineEmits<{ close: [] }>();

const parts = computed(() => splitHighlight(props.message.content, props.snippet));
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="modal">
      <div class="modal-head">
        <div class="modal-title">{{ title || "原始消息" }}</div>
        <button class="text-btn" @click="emit('close')">关闭</button>
      </div>
      <div class="modal-body">
        <div class="msg-detail-meta">
          {{ message.groupName }} · {{ message.senderName }} ·
          {{ formatTime(message.createdAt) }} · {{ TYPE_LABEL[message.type] }}
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
        <div v-if="message.content" class="msg-detail-content">
          <template v-for="(part, index) in parts" :key="index">
            <mark v-if="part.mark">{{ part.text }}</mark>
            <template v-else>{{ part.text }}</template>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

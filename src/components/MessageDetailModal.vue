<script setup lang="ts">
import { computed, ref } from "vue";
import type { Message } from "../types";
import { formatTime, splitHighlight, TYPE_LABEL } from "../utils";
import { replaceWechatEmoji } from "../wechatEmoji";
import ImageLightbox from "./ImageLightbox.vue";
import MessageEmoticon from "./MessageEmoticon.vue";
import MessageImage from "./MessageImage.vue";

const props = defineProps<{
  message: Message;
  snippet?: string | null;
  title?: string;
}>();

const emit = defineEmits<{ close: [] }>();

const parts = computed(() => splitHighlight(props.message.content, props.snippet));
const previewMd5 = ref("");
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
          引用 {{ message.quotedSender }}：{{ replaceWechatEmoji(message.quotedContent) }}
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
          v-if="
            message.content &&
            !(message.type === 'emoticon' && message.emoticonMd5)
          "
          class="msg-detail-content"
        >
          <template v-for="(part, index) in parts" :key="index">
            <mark v-if="part.mark">{{ part.text }}</mark>
            <template v-else>{{ part.text }}</template>
          </template>
        </div>
      </div>
    </div>
    <ImageLightbox v-if="previewMd5" :md5="previewMd5" @close="previewMd5 = ''" />
  </div>
</template>

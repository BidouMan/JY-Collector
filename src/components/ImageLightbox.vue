<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { loadImageUrl } from "../stores/images";

const props = defineProps<{ md5: string }>();
const emit = defineEmits<{ close: [] }>();

const src = ref("");
const failed = ref(false);

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") emit("close");
}

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  try {
    src.value = await loadImageUrl(props.md5, true);
  } catch {
    failed.value = true;
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div class="image-lightbox" @click.self="emit('close')">
    <img v-if="src" :src="src" alt="原图" />
    <div v-else class="image-lightbox-hint">
      {{ failed ? "原图无法显示" : "原图加载中…" }}
    </div>
    <button class="text-btn image-lightbox-close" @click="emit('close')">关闭</button>
  </div>
</template>

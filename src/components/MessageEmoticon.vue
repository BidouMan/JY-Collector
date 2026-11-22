<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { loadEmoticonUrl } from "../stores/images";

const props = defineProps<{
  messageId: string;
  md5?: string | null;
}>();

const root = ref<HTMLElement | null>(null);
const src = ref("");
const loading = ref(false);
const failed = ref(false);
let observer: IntersectionObserver | null = null;

async function load() {
  if (src.value || loading.value) return;
  loading.value = true;
  failed.value = false;
  try {
    src.value = await loadEmoticonUrl(props.messageId, props.md5);
  } catch {
    failed.value = true;
  } finally {
    loading.value = false;
  }
}

function startObserve() {
  if (!root.value) return;
  observer?.disconnect();
  observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        observer?.disconnect();
        observer = null;
        void load();
      }
    },
    { rootMargin: "300px" },
  );
  observer.observe(root.value);
}

onMounted(startObserve);

watch(
  () => props.messageId,
  () => {
    src.value = "";
    failed.value = false;
    startObserve();
  },
);

onBeforeUnmount(() => {
  observer?.disconnect();
});
</script>

<template>
  <div ref="root" class="message-emoticon" :class="{ loaded: !!src, failed }">
    <img v-if="src" :src="src" alt="表情" draggable="false" />
    <span v-else class="message-emoticon-hint">
      {{ failed ? "[表情]" : loading ? "加载表情…" : "[表情]" }}
    </span>
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { cachedImageUrl, loadImageUrl } from "../stores/images";

const props = defineProps<{
  md5: string;
  /** 是否直接加载原图（弹窗里可用） */
  full?: boolean;
}>();

const emit = defineEmits<{ preview: [md5: string] }>();

const root = ref<HTMLElement | null>(null);
const src = ref("");
const loading = ref(false);
const failed = ref(false);
let observer: IntersectionObserver | null = null;

async function load() {
  if (!props.md5 || src.value || loading.value) return;
  const cached = cachedImageUrl(props.md5, props.full);
  if (cached) {
    src.value = cached;
    return;
  }
  loading.value = true;
  failed.value = false;
  try {
    src.value = await loadImageUrl(props.md5, props.full);
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
  () => props.md5,
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
  <div
    ref="root"
    class="message-image"
    :class="{ loaded: !!src, failed }"
    role="button"
    tabindex="0"
    @click.stop="src && emit('preview', md5)"
    @keydown.enter.stop="src && emit('preview', md5)"
  >
    <img v-if="src" :src="src" alt="聊天图片" draggable="false" />
    <span v-else class="message-image-hint">
      {{ failed ? "图片无法显示" : loading ? "加载图片…" : "图片" }}
    </span>
  </div>
</template>

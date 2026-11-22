<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type CSSProperties } from "vue";

interface DropdownOption {
  value: string;
  label: string;
}

const props = withDefaults(
  defineProps<{
    modelValue: string;
    options: DropdownOption[];
    placeholder?: string;
    disabled?: boolean;
    compact?: boolean;
  }>(),
  {
    placeholder: "请选择",
    disabled: false,
    compact: false,
  },
);

const emit = defineEmits<{
  "update:modelValue": [value: string];
  change: [value: string];
}>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);
const triggerRef = ref<HTMLButtonElement | null>(null);
const menuRef = ref<HTMLElement | null>(null);
const menuStyle = ref<CSSProperties>({});

const currentLabel = computed(
  () => props.options.find((item) => item.value === props.modelValue)?.label ?? props.placeholder,
);

// 菜单挂到 body 并用 fixed 定位，避免被父级 overflow 裁切。
function updatePosition() {
  const rect = triggerRef.value?.getBoundingClientRect();
  if (!rect) return;
  menuStyle.value = {
    position: "fixed",
    top: `${Math.round(rect.bottom + 4)}px`,
    left: `${Math.round(rect.left)}px`,
    minWidth: `${Math.round(rect.width)}px`,
  };
}

watch(open, async (value) => {
  if (!value) return;
  updatePosition();
  await nextTick();
  updatePosition();
});

function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
}

function pick(option: DropdownOption) {
  open.value = false;
  if (option.value === props.modelValue) return;
  emit("update:modelValue", option.value);
  emit("change", option.value);
}

function onDocPointerDown(event: MouseEvent) {
  const target = event.target as Node;
  if (root.value?.contains(target) || menuRef.value?.contains(target)) return;
  open.value = false;
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") open.value = false;
}

function onViewportChange() {
  if (open.value) updatePosition();
}

onMounted(() => {
  document.addEventListener("mousedown", onDocPointerDown);
  document.addEventListener("keydown", onKeydown);
  window.addEventListener("resize", onViewportChange);
  window.addEventListener("scroll", onViewportChange, true);
});

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDocPointerDown);
  document.removeEventListener("keydown", onKeydown);
  window.removeEventListener("resize", onViewportChange);
  window.removeEventListener("scroll", onViewportChange, true);
});
</script>

<template>
  <div ref="root" class="dropdown-select" :class="{ 'is-open': open, compact }">
    <button
      ref="triggerRef"
      type="button"
      class="field-input dropdown-trigger"
      :class="{ compact }"
      :disabled="disabled"
      @click="toggle"
    >
      <span class="dropdown-label">{{ currentLabel }}</span>
      <svg class="dropdown-caret" viewBox="0 0 16 16" aria-hidden="true">
        <path
          d="M4 6.5 8 10.5 12 6.5"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>
    <Teleport to="body">
      <div v-if="open" ref="menuRef" class="dropdown-menu" :style="menuStyle" role="listbox">
        <button
          v-for="option in options"
          :key="option.value"
          type="button"
          class="dropdown-option"
          :class="{ selected: option.value === modelValue }"
          @click="pick(option)"
        >
          {{ option.label }}
        </button>
        <div v-if="options.length === 0" class="dropdown-empty">暂无选项</div>
      </div>
    </Teleport>
  </div>
</template>

import { computed, reactive } from "vue";
import { api } from "../api";
import type { Message } from "../types";

/** 跨群共用的选料篮子：状态放在这里，切换群/人员时不会丢。 */
export const basket = reactive({
  items: [] as Message[],
  loaded: false,
  open: false,
});

export const basketIdSet = computed(() => new Set(basket.items.map((item) => item.id)));

export async function refreshBasket() {
  try {
    basket.items = await api.listBasket();
  } catch {
    basket.items = [];
  }
  basket.loaded = true;
}

export async function toggleBasket(message: Message) {
  if (basketIdSet.value.has(message.id)) {
    await api.removeFromBasket([message.id]);
    basket.items = basket.items.filter((item) => item.id !== message.id);
  } else {
    await api.addToBasket([message.id]);
    basket.items = [message, ...basket.items];
  }
}

export async function removeFromBasket(ids: string[]) {
  if (!ids.length) return;
  await api.removeFromBasket(ids);
  const dropping = new Set(ids);
  basket.items = basket.items.filter((item) => !dropping.has(item.id));
}

export async function clearBasketNow() {
  await api.clearBasket();
  basket.items = [];
}

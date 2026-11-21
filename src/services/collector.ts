import { api } from "../api";
import type { Group, Message, Person } from "../types";

/**
 * 采集适配器：界面只依赖这一层。
 *
 * 现在的数据来自真实微信：Rust 后端启动时（以及点「刷新」时）调用
 * Python sidecar 解密微信本地库，归一化后写入应用自己的 SQLite；
 * 本适配器仍然只是读取应用库，所以接口和界面都不需要变。
 * 将来若要换数据来源（例如 Windows 侧直连），替换 `collector` 实例即可。
 */
export interface CollectorAdapter {
  id: string;
  label: string;
  listGroups(): Promise<Group[]>;
  listMessages(groupId: string): Promise<Message[]>;
  searchPeople(keyword: string): Promise<Person[]>;
  listPersonMessages(senderId: string): Promise<Message[]>;
  getMessage(messageId: string): Promise<Message | null>;
}

class LocalCollectorAdapter implements CollectorAdapter {
  id = "wechat";
  label = "微信采集库";

  listGroups() {
    return api.listGroups(true);
  }

  listMessages(groupId: string) {
    return api.listMessages(groupId);
  }

  searchPeople(keyword: string) {
    return api.searchPeople(keyword, true);
  }

  listPersonMessages(senderId: string) {
    return api.listPersonMessages(senderId);
  }

  getMessage(messageId: string) {
    return api.getMessage(messageId);
  }
}

export const collector: CollectorAdapter = new LocalCollectorAdapter();

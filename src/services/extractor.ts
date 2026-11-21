import type { FieldSchema, Message, Template } from "../types";
import type { ModelRow } from "./modelExtract";

export interface ExtractionSource {
  messageId: string;
  snippet: string;
}

export interface ExtractedField {
  key: string;
  label: string;
  fieldType: FieldSchema["fieldType"];
  required: boolean;
  example: string;
  value: string;
  source: ExtractionSource | null;
}

export interface ExtractInput {
  messages: Message[];
  schema: FieldSchema[];
}

export interface Extractor {
  id: string;
  label: string;
  extract(input: ExtractInput): Promise<ExtractedField[]>;
}

type TextRule = (text: string) => string | null;

function first(text: string, pattern: RegExp): string | null {
  const match = text.match(pattern);
  return match && match[1] ? match[1].trim() : null;
}

function sentences(text: string): string[] {
  return text
    .split(/[。；;\n]/)
    .map((part) => part.trim())
    .filter(Boolean);
}

function sentenceWith(text: string, keywords: string[]): string | null {
  const hit = sentences(text).find((sentence) =>
    keywords.some((keyword) => sentence.includes(keyword)),
  );
  return hit ?? null;
}

function snippetFor(text: string, value: string): string {
  const hit = sentences(text).find((sentence) => sentence.includes(value));
  return hit ?? text.trim();
}

function clean(value: string | null | undefined): string {
  if (!value) return "";
  return value
    .trim()
    .replace(/^[是为按：:\s]+/, "")
    .replace(/[，。；;、\s]+$/, "");
}

function cleanAddress(value: string | null): string {
  return clean(value)
    .replace(/^(?:还是|统计[：:]?|是|为)+/, "")
    .replace(/(?:登记|上课|报名|即可|为准)$/, "")
    .trim();
}

const NAME_STOP = /[的了费用缴收到在说请个月天这那个还就都已将把被和与及们我你他她它]/;
const NON_NAME = /^(登记|报名|电话|年级|学校|科目|家长|老师|学员|同学|姓名|地址|备注)$/;

function validName(value: string): boolean {
  if (value.length < 2 || value.length > 4) return false;
  if (NAME_STOP.test(value)) return false;
  return !NON_NAME.test(value);
}

function normalizeDate(value: string): string {
  const match = value.match(/(\d{4})\D+(\d{1,2})\D+(\d{1,2})/);
  if (!match) return value;
  return `${match[1]}-${match[2].padStart(2, "0")}-${match[3].padStart(2, "0")}`;
}

function payStatus(text: string): string | null {
  if (/(待缴|未缴|欠费)/.test(text)) return "待缴";
  if (/(已缴|已缴|已付|已转账|已交)/.test(text)) return "已缴";
  return null;
}

const RULES: Record<string, TextRule[]> = {
  student: [
    (text) => first(text, /(?:学员姓名|姓名)[：:\s]+([\u4e00-\u9fa5]{2,4})/),
    (text) =>
      first(
        text,
        /([\u4e00-\u9fa5]{2,3})(?:同学|今天|明天|周六|周日|这次|作业|报名|上课|请假|调到|恢复|能|想)/,
      ),
    (text) => first(text, /([\u4e00-\u9fa5]{2,3})(?:妈妈|爸爸|家长)/),
    (text) => first(text, /(?:[:：]\d{2}\s+)([\u4e00-\u9fa5]{2,3})/),
  ],
  grade: [
    (text) => first(text, /([一二三四五六七八九]年级|初[一二三]|高[一二三])/),
  ],
  school: [
    (text) =>
      first(
        text,
        /(?:学校|就读)[：:\s]*([\u4e00-\u9fa5]{2,12}(?:小学|中学|附小|附中|学校)?)/,
      ),
    (text) => first(text, /([\u4e00-\u9fa5]{2,10}(?:小学|中学|附小|附中|实验学校))/),
  ],
  subject: [
    (text) => first(text, /(语文|数学|英语|物理|化学|生物|历史|地理|政治|奥数|编程|书法|钢琴)/),
  ],
  fee: [
    (text) => first(text, /(?:课时费|学费)[^0-9]{0,8}(\d+(?:\.\d+)?)/),
  ],
  monthly_fee: [
    (text) =>
      first(text, /(?:晚托费|晚托费用|每月费用|月费|托管费)[^0-9]{0,10}(\d+(?:\.\d+)?)/),
  ],
  parent: [
    (text) => first(text, /(?:家长|联系人)[：:\s]*([\u4e00-\u9fa5]{2,4})/),
    (text) => first(text, /([\u4e00-\u9fa5]{2,3}(?:女士|先生|奶奶|爷爷))/),
  ],
  phone: [
    (text) => first(text, /(1[3-9]\d\*{2,6}\d{2,4}|1[3-9]\d{9})/),
  ],
  address: [
    (text) =>
      cleanAddress(
        first(
          text,
          /((?:[\u4e00-\u9fa5]{2,10}(?:路|街|道|巷|花园|小区|苑|城|名苑|大厦|公寓))[^，。；;\n]{0,30})/,
        ),
      ),
    (text) =>
      cleanAddress(
        first(
          text,
          /(?:家庭住址|接送地址|住址|地址)[：:\s]*(?:还是|是|为)?\s*([^，。；;\n]{4,40})/,
        ),
      ),
  ],
  pickup_address: [
    (text) =>
      cleanAddress(
        first(
          text,
          /((?:[\u4e00-\u9fa5]{2,10}(?:路|街|道|花园|小区|苑|城|名苑|大厦|公寓))[^，。；;\n]{0,30})/,
        ),
      ),
    (text) =>
      cleanAddress(
        first(
          text,
          /(?:接送地址|家庭住址|地址)[：:\s]*(?:还是|是|为)?\s*([^，。；;\n]{4,40})/,
        ),
      ),
  ],
  lesson_time: [
    (text) =>
      first(text, /((?:周|星期)[一二三四五六日天][^，。；;\n]{0,16}?\d{1,2}[:：]\d{2})/),
    (text) =>
      first(
        text,
        /((?:周|星期)[一二三四五六日天]\s*(?:上午|下午|晚上)?\s*\d{1,2}[:：]\d{2})/,
      ),
  ],
  enroll_date: [
    (text) =>
      first(text, /(?:报名日期|报名)[：:\s]*(\d{4}[-年]\d{1,2}[-月]\d{1,2}日?)/),
    (text) => first(text, /(20\d{2}[-/年]\d{1,2}[-/月]\d{1,2}日?)/),
  ],
  teacher: [(text) => first(text, /([\u4e00-\u9fa5]老师)/)],
  pay_status: [(text) => payStatus(text)],
  homework: [
    (text) =>
      sentenceWith(text, ["作业"]),
  ],
  weak_point: [
    (text) =>
      first(text, /(?:薄弱知识点|薄弱点|薄弱)(?:还是|是|为)?[：:\s]*([^，。；;\n]{2,20})/),
    (text) => first(text, /([\u4e00-\u9fa5]{2,10})(?:还是|比较)不稳/),
  ],
  parent_feedback: [
    (text) => sentenceWith(text, ["希望", "建议", "反馈", "麻烦"]),
  ],
  note: [
    (text) =>
      first(text, /备注[：:\s]*([^。\n]{2,60})/),
    (text) => sentenceWith(text, ["请假", "顺延", "改到", "提醒", "备注"]),
  ],
};

export class RuleBasedExtractor implements Extractor {
  id = "rule";
  label = "本地规则抽取";

  async extract({ messages, schema }: ExtractInput): Promise<ExtractedField[]> {
    const ordered = [...messages].sort((a, b) =>
      a.createdAt.localeCompare(b.createdAt),
    );

    return schema.map((field) => {
      const result: ExtractedField = {
        key: field.key,
        label: field.label,
        fieldType: field.fieldType,
        required: field.required,
        example: field.example,
        value: "",
        source: null,
      };

      const labelPattern = field.label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const rules: TextRule[] = [
        ...(RULES[field.key] ?? []),
        (text) => first(text, new RegExp(`${labelPattern}[：:]\\s*([^，。；;\\n]{1,40})`)),
      ];

      for (const message of ordered) {
        const text = message.content;
        if (!text) continue;
        for (const rule of rules) {
          const raw = rule(text);
          const value = clean(raw);
          if (!value) continue;
          if (field.key === "student" && !validName(value)) continue;
          result.value =
            field.fieldType === "number"
              ? value.match(/\d+(?:\.\d+)?/)?.[0] ?? value
              : field.fieldType === "date"
                ? normalizeDate(value)
                : value;
          result.source = {
            messageId: message.id,
            snippet: snippetFor(text, value),
          };
          break;
        }
      }

      return result;
    });
  }
}

/**
 * 模型抽取占位实现。接入真实 API 时，只需在此处请求模型并按 schema 返回字段，
 * 界面和记录回链逻辑不需要改动。
 */
export class ModelExtractor implements Extractor {
  id = "model";
  label = "模型抽取";

  constructor(private readonly getConfig: () => { apiKey: string; modelName: string }) {}

  async extract(): Promise<ExtractedField[]> {
    const { apiKey, modelName } = this.getConfig();
    if (!apiKey) {
      throw new Error("请先在设置中填写模型 API Key");
    }
    throw new Error(`模型抽取接口尚未接入（${modelName}）`);
  }
}

export const extractor = new RuleBasedExtractor();

/* ---------- 导出配置：规则抽取 + 多设备拆行 ---------- */

export interface LedgerDraft {
  messageId: string;
  groupId: string;
  title: string;
  data: Record<string, string>;
  sources: { messageId: string; fieldKey: string; snippet: string }[];
}

interface DeviceItem {
  name: string;
  quantity: string;
}

interface ChannelFields {
  hospital: string;
  dealer: string;
  payment: string;
  approval: string;
  devices: DeviceItem[];
}

const CN_NUM: Record<string, number> = {
  一: 1,
  二: 2,
  两: 2,
  三: 3,
  四: 4,
  五: 5,
  六: 6,
  七: 7,
  八: 8,
  九: 9,
  十: 10,
};

function stripEnds(value: string): string {
  return value
    .replace(/^[是为：:\s]+/, "")
    .replace(/[、，,。；;：:\s]+$/, "")
    .trim();
}

function firstLine(text: string): string {
  return (
    text
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find(Boolean) ?? ""
  );
}

function sentenceFor(text: string, value: string): string {
  const line = text
    .split(/\r?\n/)
    .map((item) => item.trim())
    .find((item) => item.includes(value));
  if (line) return line;
  const sentence = text
    .split(/[。；;]/)
    .map((item) => item.trim())
    .find((item) => item.includes(value));
  return sentence ?? value;
}

function parseHospital(text: string): string {
  const line = firstLine(text);
  if (line.includes("医院")) {
    const match = line.match(/([\u4e00-\u9fa5A-Za-z0-9（）()·/\-]{2,60}医院)/);
    return stripEnds(match ? match[1] : line);
  }
  const match = text.match(/([\u4e00-\u9fa5]{2,20}(?:医院|卫生院|医学中心|医疗中心))/);
  return match ? stripEnds(match[1]) : "";
}

function parseDealer(text: string): string {
  const explicit = text.match(/经销商[：:]\s*([^\n，。；;、]+)/);
  if (explicit) return stripEnds(explicit[1]);
  const company = text.match(
    /([\u4e00-\u9fa5（）()]{2,40}(?:有限公司|有限责任公司|股份有限公司|公司))/,
  );
  return company ? stripEnds(company[1]) : "";
}

function parseDevices(text: string): DeviceItem[] {
  let segment = "";
  const index = text.indexOf("申请绿通");
  if (index >= 0) {
    segment = text.slice(index + 4);
  } else {
    const alt = text.indexOf("绿通");
    if (alt >= 0) segment = text.slice(alt + 2);
  }
  segment = (segment.split(/[，,。；;\n]/)[0] ?? "").replace(/^[：:\s]+/, "");
  if (!segment.trim()) return [];

  return segment
    .split(/(?:和|、|及|＋|\+)/)
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => {
      const quantityMatch = part.match(/(\d+|[一二两三四五六七八九十])\s*(台|套|个|件|项)/);
      let quantity = "";
      let name = part;
      if (quantityMatch) {
        const raw = quantityMatch[1];
        const number = CN_NUM[raw] ?? Number(raw);
        quantity = Number.isFinite(number) ? String(number) : "";
        name = part.replace(quantityMatch[0], "").trim();
      }
      name = stripEnds(name)
        .replace(/(?:设备|系统)$/, "")
        .trim();
      if (!name) name = stripEnds(part);
      return { name, quantity };
    })
    .filter((item) => item.name || item.quantity);
}

function parseApproval(text: string): string {
  if (/已经?[过经]*审批|已审批|已获批|审批通过/.test(text)) return "已审批";
  if (/待审批|待批/.test(text)) return "待审批";
  if (/审批中/.test(text)) return "审批中";
  if (/未审批|未通过/.test(text)) return "未审批";
  return "";
}

function parsePayment(text: string): string {
  const match = text.match(/(分期付款|分期|全款|预付|账期|月结|一次性付款|货到付款)/);
  return match ? match[1] : "";
}

function parseChannelFields(text: string): ChannelFields {
  return {
    hospital: parseHospital(text),
    dealer: parseDealer(text),
    payment: parsePayment(text),
    approval: parseApproval(text),
    devices: parseDevices(text),
  };
}

function genericValue(text: string, label: string): string {
  if (!label) return "";
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = text.match(new RegExp(`${escaped}[：:]\\s*([^\\n，。；;]{1,40})`));
  return match ? match[1].trim() : "";
}

/** 这些列由系统生成，不参与原文回链。 */
const RESERVED_KEYS = ["sender", "forwarder", "source_group", "message_time", "excerpt"];

/** 由程序直接填的系统列，不需要发给模型，也不消耗 token。 */
function systemFieldValue(key: string, message: Message): string | null {
  switch (key) {
    case "sender":
      return message.senderName;
    case "forwarder":
      return message.quotedSender ?? "";
    case "source_group":
      return message.groupName;
    case "message_time":
      return message.createdAt;
    case "excerpt":
      return (message.content ?? "").trim();
    default:
      return null;
  }
}

/**
 * 列名/提示里出现的关键词决定这一列对应哪类信息。
 * 这样从 Excel 导入后 key 变成 col_1、col_2 时也能正确取到值。
 */
function semanticValue(
  column: { label: string },
  fields: ChannelFields,
  device: DeviceItem,
): string {
  const tag = column.label;
  if (/医院|客户|单位|机构|门店/.test(tag)) return fields.hospital;
  if (/经销商|代理|供应商|渠道/.test(tag)) return fields.dealer;
  if (/数量|台数|套数/.test(tag)) return device.quantity;
  if (/产品|设备|型号|器械|品名/.test(tag)) return device.name;
  if (/付款|支付|结算|账期|回款/.test(tag)) return fields.payment;
  if (/审批|审核/.test(tag)) return fields.approval;
  return "";
}

function titleFor(message: Message, template: Template, data: Record<string, string>): string {
  const pick = (pattern: RegExp) => {
    const column = template.columns.find((item) => pattern.test(item.label));
    return column ? data[column.key] ?? "" : "";
  };
  const parts = [pick(/医院|客户|单位|机构/), pick(/产品|设备|型号|器械/)].filter(Boolean);
  if (parts.length > 0) return parts.join(" · ");
  const first = template.columns
    .map((column) => data[column.key])
    .find((value) => Boolean(value && value.trim()));
  return first || message.groupName;
}

function buildDraft(
  message: Message,
  template: Template,
  data: Record<string, string>,
): LedgerDraft {
  const text = (message.content ?? "").trim();
  if (!data.excerpt && text) data.excerpt = text;

  const sources: LedgerDraft["sources"] = [];
  for (const column of template.columns) {
    const value = data[column.key];
    if (!value || RESERVED_KEYS.includes(column.key)) continue;
    sources.push({
      messageId: message.id,
      fieldKey: column.key,
      snippet: sentenceFor(text, value),
    });
  }

  return {
    messageId: message.id,
    groupId: message.groupId,
    title: titleFor(message, template, data),
    data,
    sources,
  };
}

function normalizeFieldValue(column: Template["columns"][number], value: string): string {
  if (column.format === "number") {
    return value.match(/-?\d+(?:\.\d+)?/)?.[0] ?? value;
  }
  if (column.format === "date") return normalizeDate(value);
  return value;
}

/**
 * 把选中的消息按配置列定义转成台账草稿；一条消息里多台设备会拆成多条。
 * 这是模型不可用时的兜底路径。
 */
export function buildLedgerDrafts(messages: Message[], template: Template): LedgerDraft[] {
  const drafts: LedgerDraft[] = [];
  const ordered = [...messages].sort((a, b) => a.createdAt.localeCompare(b.createdAt));

  for (const message of ordered) {
    const text = message.content ?? "";
    const fields = parseChannelFields(text);
    const devices: DeviceItem[] =
      fields.devices.length > 0 ? fields.devices : [{ name: "", quantity: "" }];

    for (const device of devices) {
      const data: Record<string, string> = {};

      for (const column of template.columns) {
        const fixed = systemFieldValue(column.key, message);
        let value = "";
        if (fixed !== null) {
          value = fixed;
        } else {
          switch (column.key) {
            case "hospital":
              value = fields.hospital;
              break;
            case "dealer":
              value = fields.dealer;
              break;
            case "device":
              value = device.name;
              break;
            case "quantity":
              value = device.quantity;
              break;
            case "payment":
              value = fields.payment;
              break;
            case "approval":
              value = fields.approval;
              break;
            default:
              value = semanticValue(column, fields, device) || genericValue(text, column.label);
          }
        }

        value = stripEnds(value);
        if (!value) continue;
        data[column.key] = value;
      }

      if (Object.keys(data).length === 0 && !text.trim()) continue;
      drafts.push(buildDraft(message, template, data));
    }
  }

  return drafts;
}

/**
 * 模型返回的行转成草稿；没有取到字段的消息收集到 missing，由调用方用本地规则补齐。
 */
export function buildDraftsFromModel(
  messages: Message[],
  rows: ModelRow[],
  template: Template,
): { drafts: LedgerDraft[]; missing: Message[] } {
  const grouped = new Map<string, Record<string, string>[]>();
  for (const row of rows) {
    const list = grouped.get(row.messageId) ?? [];
    list.push(row.values);
    grouped.set(row.messageId, list);
  }

  const drafts: LedgerDraft[] = [];
  const missing: Message[] = [];
  const ordered = [...messages].sort((a, b) => a.createdAt.localeCompare(b.createdAt));

  for (const message of ordered) {
    const list = grouped.get(message.id);
    if (!list || list.length === 0) {
      missing.push(message);
      continue;
    }

    let produced = 0;
    for (const values of list) {
      const data: Record<string, string> = {};
      for (const column of template.columns) {
        const fixed = systemFieldValue(column.key, message);
        if (fixed !== null) {
          if (fixed) data[column.key] = fixed;
          continue;
        }
        const raw = (values[column.key] ?? "").trim();
        const value = stripEnds(normalizeFieldValue(column, raw));
        if (value) data[column.key] = value;
      }
      if (Object.keys(data).length === 0) continue;
      drafts.push(buildDraft(message, template, data));
      produced += 1;
    }
    if (produced === 0) missing.push(message);
  }

  return { drafts, missing };
}

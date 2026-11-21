//! 模型字段抽取：把选中的微信消息与表格列定义发给大模型，返回按列 key 组织的数据行。
//!
//! 使用 OpenAI 兼容的 `/chat/completions` 协议，DeepSeek、OpenAI 及其它兼容服务都能接入。
//! API Key 只在 Rust 侧读取并放进请求头，不会进入前端、日志或仓库。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractColumnInput {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractMessageInput {
    pub id: String,
    #[serde(default)]
    pub content: String,
    /// 只有消息确实带引用时才发送，避免空字段占用 token。
    #[serde(default)]
    pub quoted_sender: Option<String>,
    #[serde(default)]
    pub quoted_content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedMessage {
    pub message_id: String,
    pub rows: Vec<HashMap<String, String>>,
}

const SYSTEM_PROMPT: &str = r#"你是医疗器械绿通申请的信息录入助手。
用户会给你一组微信群消息和一份表格列定义，请从每条消息中抽取列定义要求的字段。

硬性要求：
1. 只输出一个 JSON 对象，不要输出解释、Markdown 或多余文字。
2. 输出结构固定为：{"messages":[{"id":"消息 id","rows":[{"字段 key":"值"}]}]}。
3. 每条消息至少输出一行；一条消息里有多台设备时，每台设备单独一行，医院、经销商、付款方式、审批状态等信息在该消息的每一行都重复填写。
4. 只使用消息原文中明确出现的信息。原文没有的字段输出空字符串，禁止推测、补全或编造。
5. 医院填写完整全称（原文中如有多个名称用原文的分隔符保留，例如“上海市浦东医院/复旦大学附属浦东医院”）。
6. 经销商填写完整公司名称，去掉“经销商”“代理”等称谓和句末标点。
7. 产品只填写设备名称或型号（例如 Opal HDx、PFA、Farastar），不要带上“设备、系统、一台”等词；数量列只填阿拉伯数字。
8. 付款方式、审批状态按原文概括，如“分期付款”“已审批”“待审批”。
9. 字段名必须严格使用列定义里的 key，不要输出列定义以外的字段。
10. 数字列只输出数字，日期列输出 YYYY-MM-DD。
11. 列定义里的 hint 是该列的填写示例，用来参考写法和粒度；示例本身不能当作消息内容照抄。"#;

/// 把用户配置的接口地址补全成 chat/completions 端点。
fn chat_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "https://api.deepseek.com/v1/chat/completions".to_string();
    }
    if trimmed.ends_with("/chat/completions") {
        return trimmed.to_string();
    }
    format!("{}/chat/completions", trimmed)
}

fn clip(text: &str, max: usize) -> String {
    let clipped: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        format!("{}…", clipped)
    } else {
        clipped
    }
}

fn strip_fence(content: &str) -> String {
    let trimmed = content.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    body.strip_suffix("```").unwrap_or(body).trim().to_string()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

/// 模型偶尔会用列名而不是 key 作字段名，这里两种都认。
fn map_row(
    row: &Value,
    columns: &[ExtractColumnInput],
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(object) = row.as_object() else {
        return out;
    };
    for (raw_key, raw_value) in object {
        let name = raw_key.trim();
        let target = columns
            .iter()
            .find(|column| column.key == name)
            .or_else(|| columns.iter().find(|column| column.label.trim() == name))
            .map(|column| column.key.clone());
        let Some(key) = target else {
            continue;
        };
        let value = value_to_string(raw_value);
        if value.is_empty() {
            continue;
        }
        out.entry(key).or_insert(value);
    }
    out
}

fn parse_model_json(
    content: &str,
    columns: &[ExtractColumnInput],
    fallback_id: Option<&str>,
) -> Result<Vec<ExtractedMessage>, String> {
    let cleaned = strip_fence(content);
    let root: Value = serde_json::from_str(&cleaned)
        .map_err(|error| format!("模型返回的内容不是有效 JSON：{}", clip(&error.to_string(), 160)))?;

    let items: Vec<&Value> = if let Some(list) = root.get("messages").and_then(|value| value.as_array())
    {
        list.iter().collect()
    } else if let Some(list) = root.as_array() {
        list.iter().collect()
    } else if root.get("rows").is_some() {
        vec![&root]
    } else {
        return Err("模型返回的 JSON 缺少 messages 字段".to_string());
    };

    let mut out = Vec::new();
    for item in items {
        let message_id = item
            .get("id")
            .or_else(|| item.get("messageId"))
            .or_else(|| item.get("message_id"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| fallback_id.map(|value| value.to_string()))
            .unwrap_or_default();
        if message_id.is_empty() {
            continue;
        }
        let rows: Vec<HashMap<String, String>> = item
            .get("rows")
            .and_then(|value| value.as_array())
            .map(|list| list.iter().map(|row| map_row(row, columns)).collect())
            .unwrap_or_default();
        let rows: Vec<HashMap<String, String>> = rows
            .into_iter()
            .filter(|row| !row.is_empty())
            .collect();
        if rows.is_empty() {
            continue;
        }
        out.push(ExtractedMessage { message_id, rows });
    }

    if out.is_empty() {
        return Err("模型没有返回可用的表格行".to_string());
    }
    Ok(out)
}

/// 从 OpenAI 兼容响应里取出 choices[0].message.content。
fn extract_content(text: &str) -> Result<String, String> {
    let payload: Value = serde_json::from_str(text)
        .map_err(|error| format!("模型响应不是有效 JSON：{}", clip(&error.to_string(), 160)))?;
    payload
        .get("choices")
        .and_then(|value| value.as_array())
        .and_then(|list| list.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| format!("模型响应里没有内容：{}", clip(text, 200)))
}

fn build_prompt(columns: &[ExtractColumnInput], messages: &[ExtractMessageInput]) -> String {
    let columns_json =
        serde_json::to_string_pretty(columns).unwrap_or_else(|_| "[]".to_string());
    let messages_json =
        serde_json::to_string_pretty(messages).unwrap_or_else(|_| "[]".to_string());
    format!(
        "表格列定义（输出字段名必须使用这里的 key）：\n{}\n\n待整理的消息：\n{}\n\n请按系统要求输出 JSON。",
        columns_json, messages_json
    )
}

async fn post_chat(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &Value,
) -> Result<reqwest::Response, String> {
    client
        .post(url)
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .map_err(|error| format!("无法连接模型接口：{}", error))
}

/// 调用模型抽取；返回按消息分组的行数据。
pub async fn call_model(
    base: &str,
    api_key: &str,
    model: &str,
    columns: &[ExtractColumnInput],
    messages: &[ExtractMessageInput],
) -> Result<Vec<ExtractedMessage>, String> {
    if api_key.trim().is_empty() {
        return Err("请先在设置 - 模型接口中填写 API Key".to_string());
    }
    if columns.is_empty() {
        return Err("当前导出配置还没有列定义".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| format!("初始化网络请求失败：{}", error))?;
    let url = chat_url(base);
    let prompt = build_prompt(columns, messages);
    // DeepSeek 的 Flash/Pro 默认开启思考模式。字段抽取是结构化整理，
    // 关闭思考可以明显降低延迟和费用；其它兼容服务不发送该参数。
    let disable_thinking = url.contains("deepseek.com");
    let make_body = |json_mode: bool| {
        let mut body = serde_json::json!({
            "model": model,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": prompt}
            ]
        });
        if json_mode {
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }
        if disable_thinking {
            body["thinking"] = serde_json::json!({"type": "disabled"});
        }
        body
    };

    let mut response = post_chat(&client, &url, api_key, &make_body(true)).await?;
    // 少数兼容服务不支持 response_format，被拒时去掉该参数再试一次。
    if response.status() == reqwest::StatusCode::BAD_REQUEST {
        response = post_chat(&client, &url, api_key, &make_body(false)).await?;
    }

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取模型响应失败：{}", error))?;
    if !status.is_success() {
        let hint = match status.as_u16() {
            401 | 403 => "，请检查 API Key 是否正确",
            404 => "，请检查接口地址与模型名称",
            429 => "，请求过于频繁，请稍后再试",
            _ => "",
        };
        return Err(format!(
            "模型接口返回 {}{}：{}",
            status.as_u16(),
            hint,
            clip(&text, 200)
        ));
    }

    let content = extract_content(&text)?;

    let fallback_id = if messages.len() == 1 {
        messages.first().map(|message| message.id.as_str())
    } else {
        None
    };
    parse_model_json(&content, columns, fallback_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<ExtractColumnInput> {
        vec![
            ExtractColumnInput {
                key: "col_1".into(),
                label: "医院".into(),
                hint: "存储医院名称".into(),
                format: "text".into(),
            },
            ExtractColumnInput {
                key: "col_3".into(),
                label: "产品".into(),
                hint: String::new(),
                format: "text".into(),
            },
        ]
    }

    #[test]
    fn normalizes_chat_url() {
        assert_eq!(
            chat_url("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url(""),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn accepts_minimal_message_payload() {
        let parsed: ExtractMessageInput =
            serde_json::from_str(r#"{"id":"m1","content":"申请绿通Opal HDx一台"}"#).unwrap();
        assert_eq!(parsed.id, "m1");
        assert!(parsed.quoted_content.is_none());
        let json = serde_json::to_string(&parsed).unwrap();
        assert!(!json.contains("groupName"));
        assert!(!json.contains("createdAt"));
    }

    #[test]
    fn parses_rows_by_key_and_label() {
        let content = r#"```json
{"messages":[{"id":"m1","rows":[
  {"col_1":"上海市浦东医院/复旦大学附属浦东医院","col_3":"Opal HDx"},
  {"医院":"浙江大学医学院附属第二医院","产品":"Farastar","未知列":"丢弃"}
]}]}
```"#;
        let parsed = parse_model_json(content, &columns(), None).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].message_id, "m1");
        assert_eq!(parsed[0].rows.len(), 2);
        assert_eq!(
            parsed[0].rows[0].get("col_1").map(String::as_str),
            Some("上海市浦东医院/复旦大学附属浦东医院")
        );
        assert_eq!(
            parsed[0].rows[1].get("col_3").map(String::as_str),
            Some("Farastar")
        );
        assert!(!parsed[0].rows[1].contains_key("未知列"));
    }

    #[test]
    fn reads_content_from_chat_response() {
        let body =
            r#"{"choices":[{"message":{"role":"assistant","content":"{\"messages\":[]}"}}]}"#;
        assert_eq!(extract_content(body).unwrap(), "{\"messages\":[]}");
        assert!(extract_content(r#"{"choices":[]}"#).is_err());
    }

    #[test]
    fn falls_back_to_message_id_when_missing() {
        let content = r#"{"rows":[{"col_1":"某某医院"}]}"#;
        let parsed = parse_model_json(content, &columns(), Some("m9")).unwrap();
        assert_eq!(parsed[0].message_id, "m9");
    }
}

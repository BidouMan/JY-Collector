//! 微信 4.x 图片消息解码。
//!
//! 图片原文在 `msg/attach/<会话md5>/<YYYY-MM>/Img/<文件md5>[_t|_h].dat`：
//! - `_t` 是缩略图，体积通常几 KB，聊天列表优先用它；
//! - 无后缀是原图，点击预览时再解。
//!
//! V2 格式（2025-08+）：15 字节头 + AES-128-ECB 区 + 明文区 + XOR 区。
//! AES key / XOR key 可由本机 uin 与 wxid 派生，无需读取微信进程内存：
//!   xor_key = uin & 0xFF
//!   aes_key = MD5(str(uin) + wxid).hex()[:16]（按 ASCII 使用）
//! uin 从 `app_data/net/kvcomm/key_<uin>_*.statistic` 文件名读取。

use std::path::{Path, PathBuf};

use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
use aes::Aes128;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

const V2_MAGIC: &[u8] = b"\x07\x08V2\x08\x07";
const V1_MAGIC: &[u8] = b"\x07\x08V1\x08\x07";
const V1_AES_KEY: &[u8; 16] = b"cfcd208495d565ef";

pub fn load_image(account: &str, md5_hex: &str, full: bool) -> Result<(String, String), String> {
    let md5_hex = md5_hex.trim().to_lowercase();
    if md5_hex.len() != 32 || !md5_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("图片标识不正确".to_string());
    }
    let account_root = crate::wechat::default_db_root().join(account);
    let path = find_image_file(&account_root, &md5_hex, full)
        .ok_or_else(|| "本地没有找到这张图片（可能未下载到电脑）".to_string())?;
    let data = std::fs::read(&path).map_err(|_| "读取图片文件失败".to_string())?;
    let decoded = decrypt_dat(&data, &account_root)?;
    let mime = image_mime(&decoded)
        .ok_or_else(|| "图片格式暂不支持预览".to_string())?;
    Ok((mime.to_string(), BASE64.encode(decoded)))
}

/// 在 msg/attach/*/*/Img 下找到指定 md5 的图片文件。
fn find_image_file(account_root: &Path, md5: &str, full: bool) -> Option<PathBuf> {
    let names: [&str; 3] = if full {
        [".dat", "_h.dat", "_t.dat"]
    } else {
        ["_t.dat", ".dat", "_h.dat"]
    };
    let attach = account_root.join("msg").join("attach");
    let mut fallback: Option<PathBuf> = None;
    let Ok(chats) = std::fs::read_dir(attach) else {
        return None;
    };
    for chat in chats.flatten() {
        let Ok(months) = std::fs::read_dir(chat.path()) else {
            continue;
        };
        for month in months.flatten() {
            let img_dir = month.path().join("Img");
            let Ok(entries) = std::fs::read_dir(&img_dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                for (index, suffix) in names.iter().enumerate() {
                    if name == format!("{md5}{suffix}") {
                        if index == 0 {
                            return Some(entry.path());
                        }
                        if fallback.is_none() {
                            fallback = Some(entry.path());
                        }
                    }
                }
            }
        }
    }
    fallback
}

fn decrypt_dat(data: &[u8], account_root: &Path) -> Result<Vec<u8>, String> {
    if data.len() >= 6 && data.starts_with(V2_MAGIC) {
        for (xor_key, aes_key) in derive_key_candidates(account_root) {
            if let Some(out) = decrypt_v2(data, &aes_key, xor_key) {
                if image_mime(&out).is_some() {
                    return Ok(out);
                }
            }
        }
        return Err("图片密钥不匹配，无法解密".to_string());
    }
    if data.len() >= 6 && data.starts_with(V1_MAGIC) {
        if let Some(out) = decrypt_v2(data, V1_AES_KEY, 0x88) {
            if image_mime(&out).is_some() {
                return Ok(out);
            }
        }
        return Err("图片解密失败".to_string());
    }
    // 更早的单字节 XOR 格式：用图片头反推 key，并用 JPEG 结尾做校验。
    if let Some(out) = decrypt_legacy_xor(data) {
        return Ok(out);
    }
    Err("无法识别的图片格式".to_string())
}

fn decrypt_v2(data: &[u8], aes_key: &[u8; 16], xor_key: u8) -> Option<Vec<u8>> {
    if data.len() < 15 {
        return None;
    }
    let aes_size = u32::from_le_bytes(data[6..10].try_into().ok()?) as usize;
    let xor_size = u32::from_le_bytes(data[10..14].try_into().ok()?) as usize;
    let aligned = if aes_size % 16 == 0 {
        aes_size + 16
    } else {
        aes_size + (16 - aes_size % 16)
    };
    let offset = 15;
    if offset + aligned > data.len() || xor_size > data.len() {
        return None;
    }
    let mut aes_plain = vec![0u8; aligned];
    let cipher = Aes128::new(GenericArray::from_slice(aes_key));
    for (index, chunk) in data[offset..offset + aligned].chunks_exact(16).enumerate() {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        aes_plain[index * 16..index * 16 + 16].copy_from_slice(&block);
    }
    // 去掉 PKCS7 填充
    if let Some(&pad) = aes_plain.last() {
        let pad = pad as usize;
        if (1..=16).contains(&pad) && pad <= aes_plain.len() {
            aes_plain.truncate(aes_plain.len() - pad);
        }
    }
    let raw_end = data.len() - xor_size;
    if offset + aligned > raw_end {
        return None;
    }
    let mut out = aes_plain;
    out.extend_from_slice(&data[offset + aligned..raw_end]);
    out.extend(data[raw_end..].iter().map(|byte| byte ^ xor_key));
    Some(out)
}

fn decrypt_legacy_xor(data: &[u8]) -> Option<Vec<u8>> {
    let magics: [&[u8]; 5] = [b"\xff\xd8\xff", b"\x89PNG", b"GIF", b"RIFF", b"II*\x00"];
    for magic in magics {
        if data.is_empty() {
            return None;
        }
        let key = data[0] ^ magic[0];
        let decoded: Vec<u8> = data.iter().map(|byte| byte ^ key).collect();
        if decoded.starts_with(magic) {
            if magic == b"\xff\xd8\xff" && !decoded.ends_with(b"\xff\xd9") {
                continue;
            }
            return Some(decoded);
        }
    }
    None
}

pub(crate) fn image_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// 枚举 (uin, wxid) 组合并派生 AES key。
fn derive_key_candidates(account_root: &Path) -> Vec<(u8, [u8; 16])> {
    let account = account_root
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut wxids = vec![account.clone()];
    if let Some(normalized) = normalize_wxid(&account) {
        if normalized != account {
            wxids.push(normalized);
        }
    }
    let mut out = Vec::new();
    for uin in read_uins() {
        for wxid in &wxids {
            let xor_key = uin.parse::<u64>().unwrap_or(0) as u8;
            let digest = format!("{:x}", md5::compute(format!("{uin}{wxid}")));
            let mut key = [0u8; 16];
            key.copy_from_slice(&digest.as_bytes()[..16]);
            out.push((xor_key, key));
        }
    }
    out
}

/// uin 缓存在 `app_data/net/kvcomm/key_<uin>_*.statistic` 文件名里。
fn read_uins() -> Vec<String> {
    let Some(root) = crate::wechat::default_db_root().parent().map(|parent| {
        parent.join("app_data").join("net").join("kvcomm")
    }) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut uins = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("key_") || !name.ends_with(".statistic") {
            continue;
        }
        let rest = &name[4..];
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() && !uins.contains(&digits) {
            uins.push(digits);
        }
    }
    uins
}

/// `missyoucex_e9c8` -> `missyoucex`；`wxid_xxx_yyyy` -> `wxid_xxx`。
fn normalize_wxid(account: &str) -> Option<String> {
    if account.is_empty() {
        return None;
    }
    if let Some(rest) = account.strip_prefix("wxid_") {
        let head = rest.split('_').next().unwrap_or(rest);
        return Some(format!("wxid_{head}"));
    }
    let bytes = account.as_bytes();
    if bytes.len() > 5 && bytes[bytes.len() - 5] == b'_' {
        let suffix = &account[account.len() - 4..];
        if suffix.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return Some(account[..account.len() - 5].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::allowed_emoticon_url;

    #[test]
    fn emoticon_url_only_allows_tencent_hosts() {
        assert!(allowed_emoticon_url(
            "http://wxapp.tc.qq.com/262/20304/stodownload?m=abc"
        ));
        assert!(allowed_emoticon_url(
            "https://mmbiz.qpic.cn/abc/def.png"
        ));
        assert!(allowed_emoticon_url(
            "https://wx.qlogo.cn/mmhead/abc/132"
        ));
        assert!(!allowed_emoticon_url("http://127.0.0.1:8080/secret"));
        assert!(!allowed_emoticon_url("http://localhost/secret"));
        assert!(!allowed_emoticon_url(
            "http://wxapp.tc.qq.com@127.0.0.1/secret"
        ));
        assert!(!allowed_emoticon_url("http://wxapp.tc.qq.com.evil.com/x"));
        assert!(!allowed_emoticon_url("file:///etc/passwd"));
    }
}

/// 表情：从消息 XML 里的地址按需下载。部分地址返回明文图片，
/// 部分需要再用消息里的 aeskey 做 AES-128-CBC 解密（IV 与 key 相同）。
/// 下载白名单内的图片（表情/头像共用），内容不落盘。
async fn fetch_allowed(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("图片地址不正确".to_string());
    }
    if !allowed_emoticon_url(url) {
        return Err("图片地址不在允许的域名内".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 {
                attempt.stop()
            } else if allowed_emoticon_url(attempt.url().as_str()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|_| "网络客户端初始化失败".to_string())?;
    let response = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|_| "图片下载失败（网络不可用）".to_string())?;
    if !response.status().is_success() {
        return Err(format!("图片下载失败（HTTP {}）", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "图片内容读取失败".to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("图片文件过大，已跳过".to_string());
    }
    Ok(bytes.to_vec())
}

pub async fn load_emoticon(
    url: &str,
    aes_key: Option<&str>,
) -> Result<(String, String), String> {
    let bytes = fetch_allowed(url).await?;
    let decoded = if image_mime(&bytes).is_some() {
        bytes
    } else {
        let key = aes_key.unwrap_or("").trim();
        if key.len() != 32 || !key.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err("表情格式暂不支持".to_string());
        }
        decrypt_emoticon_cbc(&bytes, key)?
    };
    let mime = image_mime(&decoded).ok_or_else(|| "表情格式暂不支持".to_string())?;
    Ok((mime.to_string(), BASE64.encode(decoded)))
}

/// 头像：联系人库里的 CDN 地址，下载后直接展示。
pub async fn load_avatar(url: &str) -> Result<(String, String), String> {
    let bytes = fetch_allowed(url).await?;
    let mime = image_mime(&bytes).ok_or_else(|| "头像格式暂不支持".to_string())?;
    Ok((mime.to_string(), BASE64.encode(bytes)))
}

/// 只允许腾讯系的图片域名，避免消息里的地址被伪造成内网/本机地址（SSRF）。
fn allowed_emoticon_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('#')
        || authority.contains('?')
        || authority.contains('\\')
    {
        return false;
    }
    let host = authority.split(':').next().unwrap_or("").to_ascii_lowercase();
    ["qq.com", "qpic.cn", "qlogo.cn", "weixin.qq.com", "wechat.com", "tencent.com", "gtimg.cn"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn decrypt_emoticon_cbc(data: &[u8], key_hex: &str) -> Result<Vec<u8>, String> {
    let key: Vec<u8> = (0..key_hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&key_hex[index..index + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(|_| "表情密钥格式不正确".to_string())?;
    if key.len() != 16 || data.len() < 16 || data.len() % 16 != 0 {
        return Err("表情加密数据不完整".to_string());
    }
    let cipher = Aes128::new(GenericArray::from_slice(&key));
    let mut previous = key.clone();
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        for index in 0..16 {
            block[index] ^= previous[index];
        }
        out.extend_from_slice(&block);
        previous = chunk.to_vec();
    }
    if let Some(&pad) = out.last() {
        let pad = pad as usize;
        if (1..=16).contains(&pad) && pad <= out.len() {
            out.truncate(out.len() - pad);
        }
    }
    Ok(out)
}

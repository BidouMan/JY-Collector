//! 微信真实数据采集（Rust 侧编排）。
//!
//! 为什么用 Python sidecar，而不是把解密逻辑用 Rust 重写：
//! 1. SQLCipher 4 页格式、微信 4.x 的 WAL 帧合并、`local_type` 高位子类型、
//!    zstd 压缩、protobuf/XML 正文解码这些细节已经在
//!    `2026-11-18/wechat-mac-decrypt-spike` 里逐项实测跑通，并有真实库验证；
//!    直接复用自己的脚本，比在 Rust 里重新实现（还要引入 sqlcipher 编译依赖）
//!    风险低得多，也更快跑通。
//! 2. Python 只负责「解密 + 读表 + 归一化成 JSON」，不碰应用数据库、不保存密钥；
//!    落库、增量游标、错误提示全部在 Rust 侧完成，前端拿到的永远只是普通消息。
//!
//! 将来如果要在 Windows 交付、去掉 Python 依赖，替换点只有这一处：
//! - 把 `run_sidecar()` 换成 Rust 实现：用 `rusqlite` 的 `sqlcipher` 特性
//!   （或 `libsqlcipher-sys`）打开库，或者照搬 `sidecar/sqlcipher4.py` + `wal.py`
//!   的页解密逻辑（AES-256-CBC + HMAC-SHA512 + WAL salt 过滤），
//!   用 `aes` / `hmac` / `sha2` / `zstd` 这几个 crate 即可；
//! - 密钥获取换成 Windows 侧从微信进程内存读取（见 `work/windows_probe` 的验证脚本）；
//! - 只要仍然返回 `models::WxExport` 同构的数据，`db::apply_wechat_sync` 和界面都不用改。
//!
//! 密钥安全：只从 macOS 钥匙串或本机凭据文件读取，经子进程 stdin 传递，
//! 不进入命令行参数、环境变量、前端 IPC 与日志。

use crate::models::{SetupStatusDto, SourceStatusDto, SyncSummaryDto, WxExport};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const KEYCHAIN_SERVICE: &str = "jycollector.wechat.key";
const KEYCHAIN_ACCOUNT: &str = "wechat-macos";

/// Windows 版尚未实现本机微信数据源（取密钥、解密、权限流程都在 macOS 侧）。
/// 返回统一的友好提示，让界面显示一条简洁说明而不是多层报错。
#[cfg(target_os = "windows")]
const SOURCE_UNAVAILABLE: Option<&str> =
    Some("Windows 版暂不支持读取本机微信数据，其他功能可正常使用");
#[cfg(not(target_os = "windows"))]
const SOURCE_UNAVAILABLE: Option<&str> = None;

pub struct SourceState {
    pub sidecar_dir: PathBuf,
    pub status: Mutex<SourceStatusDto>,
    /// 上次成功同步时的消息库指纹（文件名 + 大小 + 修改时间）。
    synced: Mutex<Option<MessageFingerprint>>,
    /// 最近一次观察到的指纹及它保持不变的起点（20 秒去抖）。
    observed: Mutex<Option<(MessageFingerprint, Instant)>>,
    /// 已经做过增量检查的指纹，避免同一次变动重复解密。
    checked: Mutex<Option<MessageFingerprint>>,
    /// 上次增量检查的时间点（30 秒最小间隔）。
    last_check: Mutex<Option<Instant>>,
    /// 已确认有关注群的新消息，等待用户刷新。
    pending: Mutex<bool>,
    /// 同一时间只允许一次增量检查。
    checking: AtomicBool,
    /// 成功同步次数，用于丢弃与同步并发的检查结果。
    generation: AtomicU64,
}

impl SourceState {
    pub fn new(sidecar_dir: PathBuf) -> Self {
        Self {
            sidecar_dir,
            status: Mutex::new(SourceStatusDto::default()),
            synced: Mutex::new(None),
            observed: Mutex::new(None),
            checked: Mutex::new(None),
            last_check: Mutex::new(None),
            pending: Mutex::new(false),
            checking: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }
}

type MessageFingerprint = Vec<(String, u64, u128)>;

/// 消息库文件停止变化多久后，才允许做一次增量解密。
const CHECK_DEBOUNCE: Duration = Duration::from_secs(20);
/// 两次增量解密之间的最小间隔。
const CHECK_MIN_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) fn default_db_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home)
        .join("Library/Containers/com.tencent.xinWeChat/Data/Documents/xwechat_files")
}

/// 本机微信账号目录（存在 contact.db 才算有效账号），只做目录检查。
pub fn data_accounts() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(default_db_root()) else {
        return Vec::new();
    };
    let mut accounts = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        if entry.path().join("db_storage/contact/contact.db").is_file() {
            accounts.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    accounts.sort();
    accounts
}

/// 向导与重新授权入口共用的检测：目录、账号、口令是否存在、口令能否解开库。
/// 只解 contact.db 首页做校验，不读消息库、不写库、不打印密钥。
pub fn setup_status(sidecar_dir: &Path) -> SetupStatusDto {
    let mut status = SetupStatusDto {
        platform: std::env::consts::OS.to_string(),
        ..Default::default()
    };
    if cfg!(target_os = "windows") {
        status.state = "unsupported".to_string();
        status.message = "Windows 版暂不支持读取本机微信数据，可以继续完成其他设置".to_string();
        return status;
    }

    status.supported = true;
    let accounts = data_accounts();
    status.account_count = accounts.len() as i64;
    status.data_found = !accounts.is_empty();
    if !status.data_found {
        status.state = "no_data".to_string();
        status.message = "还没有找到本机微信聊天记录，请先在本机登录微信".to_string();
        return status;
    }

    let key_hex = match load_key_hex() {
        Ok(key) => {
            status.key_present = true;
            key
        }
        Err(_) => {
            status.state = "needs_auth".to_string();
            status.message = "找到了微信聊天记录，但还没有读取授权".to_string();
            return status;
        }
    };

    let request = serde_json::json!({
        "probe": "status",
        "keyHex": key_hex,
        "dbRoot": default_db_root().to_string_lossy(),
    });
    match run_sidecar_json(sidecar_dir, &request) {
        Ok(value) => {
            let account = value
                .get("account")
                .and_then(|item| item.as_str())
                .unwrap_or_default();
            if account.is_empty() {
                status.state = "needs_auth".to_string();
                status.message = "读取授权已失效，需要重新授权".to_string();
            } else {
                status.authorized = true;
                status.account = account.to_string();
                status.state = "ready".to_string();
                status.message = "已找到微信数据，可以开始同步".to_string();
            }
        }
        Err(_) => {
            status.state = "error".to_string();
            status.message = "暂时无法读取微信数据，请确认微信已经登录后重试".to_string();
        }
    }
    status
}

/// 同步失败是否需要用户重新走向导（口令失效、换账号、数据目录变化等）。
pub fn is_auth_error(message: &str) -> bool {
    [
        "口令",
        "密钥",
        "校验失败",
        "授权",
        "数据目录",
        "账号目录",
        "未找到微信数据",
    ]
    .iter()
    .any(|word| message.contains(word))
}

/// 主账号下的会话库；账号未知或文件不存在时返回 None。
fn session_file(account: &str) -> Option<PathBuf> {
    if account.is_empty() {
        return None;
    }
    let path = default_db_root()
        .join(account)
        .join("db_storage")
        .join("session")
        .join("session.db");
    path.is_file().then_some(path)
}

/// 只 stat 指定库文件及其 -wal，不做任何解密或读取。
fn fingerprint_file(path: &Path) -> MessageFingerprint {
    let mut candidates = vec![path.to_path_buf()];
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    candidates.push(PathBuf::from(wal));

    let mut files = Vec::new();
    for candidate in candidates {
        let Ok(meta) = std::fs::metadata(&candidate) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let name = candidate
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|gap| gap.as_nanos())
            .unwrap_or(0);
        files.push((name, meta.len(), modified));
    }
    files.sort();
    files
}

/// 记录一次成功同步后的基线；会话库不可用时置空，后续检查保持静默。
pub fn mark_synced(source: &SourceState, account: &str) {
    let current = session_file(account).map(|path| fingerprint_file(&path));
    if let Ok(mut synced) = source.synced.lock() {
        *synced = current.clone();
    }
    if let Ok(mut observed) = source.observed.lock() {
        *observed = current.map(|value| (value, Instant::now()));
    }
    if let Ok(mut checked) = source.checked.lock() {
        *checked = None;
    }
    if let Ok(mut last_check) = source.last_check.lock() {
        *last_check = None;
    }
    if let Ok(mut pending) = source.pending.lock() {
        *pending = false;
    }
    source.generation.fetch_add(1, Ordering::SeqCst);
}

pub fn is_pending(source: &SourceState) -> bool {
    source.pending.lock().map(|value| *value).unwrap_or(false)
}

pub fn set_pending(source: &SourceState, value: bool) -> bool {
    if let Ok(mut pending) = source.pending.lock() {
        *pending = value;
    }
    value
}

pub fn current_generation(source: &SourceState) -> u64 {
    source.generation.load(Ordering::SeqCst)
}

pub fn account(source: &SourceState) -> String {
    source
        .status
        .lock()
        .map(|status| status.account.clone())
        .unwrap_or_default()
}

pub fn release_check(source: &SourceState) {
    source.checking.store(false, Ordering::Release);
}

/// 是否满足触发条件：文件变动已静默 20 秒、距上次检查 ≥30 秒、
/// 且这一次变动还没检查过。满足时认领一次检查并返回 true，
/// 调用方结束后必须调用 `release_check`。
pub fn claim_check(source: &SourceState) -> bool {
    if source.checking.swap(true, Ordering::AcqRel) {
        return false;
    }
    if eligible_for_check(source) {
        return true;
    }
    release_check(source);
    false
}

fn eligible_for_check(source: &SourceState) -> bool {
    let account = account(source);
    let synced = match source.synced.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return false,
    };
    let Some(synced) = synced else {
        return false;
    };
    let Some(path) = session_file(&account) else {
        return false;
    };
    let current = fingerprint_file(&path);
    let now = Instant::now();

    // 记录 stat 变化时间：内容一变就把去抖计时器重置。
    if let Ok(mut observed) = source.observed.lock() {
        let unchanged = observed
            .as_ref()
            .map(|(value, _)| value == &current)
            .unwrap_or(false);
        if !unchanged {
            *observed = Some((current.clone(), now));
        }
    }

    let stable_for = match source.observed.lock() {
        Ok(observed) => observed
            .as_ref()
            .map(|(_, at)| at.elapsed())
            .unwrap_or_default(),
        Err(_) => return false,
    };
    if stable_for < CHECK_DEBOUNCE {
        return false;
    }
    if current == synced {
        return false;
    }
    if source
        .checked
        .lock()
        .map(|checked| checked.as_ref() == Some(&current))
        .unwrap_or(false)
    {
        return false;
    }
    if let Ok(last_check) = source.last_check.lock() {
        if let Some(at) = *last_check {
            if at.elapsed() < CHECK_MIN_INTERVAL {
                return false;
            }
        }
    }

    // 认领这次变动，后续轮询不会重复解密。
    if let Ok(mut checked) = source.checked.lock() {
        *checked = Some(current);
    }
    if let Ok(mut last_check) = source.last_check.lock() {
        *last_check = Some(now);
    }
    true
}

/// 只解 session.db，读取关注会话的最后消息时间（不读消息库、不写库）。
pub fn session_last_timestamps(
    sidecar_dir: &Path,
    account: &str,
    watched: &[String],
) -> Result<HashMap<String, i64>, String> {
    if let Some(message) = SOURCE_UNAVAILABLE {
        return Err(message.to_string());
    }
    let key_hex = load_key_hex()?;
    let request = serde_json::json!({
        "probe": "session",
        "keyHex": key_hex,
        "dbRoot": default_db_root().to_string_lossy(),
        "account": account,
        "usernames": watched,
    });
    let value = run_sidecar_json(sidecar_dir, &request)?;
    let stamps = value
        .get("lastTimestamps")
        .cloned()
        .ok_or_else(|| "会话库结果缺少字段".to_string())?;
    serde_json::from_value(stamps).map_err(|_| "会话库结果解析失败".to_string())
}

pub struct AvatarSnapshot {
    pub avatars: Vec<crate::models::WxAvatarInput>,
    pub urls: Vec<crate::models::WxAvatarUrlInput>,
}

/// 只解头像库与联系人库，返回本地头像与头像 CDN 地址（不读消息库、不写库）。
pub fn avatar_snapshot(sidecar_dir: &Path, account: &str) -> Result<AvatarSnapshot, String> {
    if let Some(message) = SOURCE_UNAVAILABLE {
        return Err(message.to_string());
    }
    let key_hex = load_key_hex()?;
    let request = serde_json::json!({
        "probe": "avatars",
        "keyHex": key_hex,
        "dbRoot": default_db_root().to_string_lossy(),
        "account": account,
    });
    let value = run_sidecar_json(sidecar_dir, &request)?;
    let avatars = value
        .get("avatars")
        .cloned()
        .ok_or_else(|| "头像结果缺少字段".to_string())?;
    let urls = value.get("avatarUrls").cloned().unwrap_or_else(|| serde_json::json!([]));
    Ok(AvatarSnapshot {
        avatars: serde_json::from_value(avatars).map_err(|_| "头像结果解析失败".to_string())?,
        urls: serde_json::from_value(urls).map_err(|_| "头像地址解析失败".to_string())?,
    })
}

fn credentials_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("JY_WECHAT_CREDENTIALS") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        Path::new(&home)
            .join("Library/Application Support/com.jy.collector/wechat-credentials.json"),
    )
}

fn valid_hex_key(raw: &str) -> bool {
    let trimmed = raw.trim().trim_start_matches("0x");
    trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
}

/// 读取口令：优先钥匙串，其次本机凭据文件。
fn load_key_hex() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("security")
            .args([
                "find-generic-password",
                "-a",
                KEYCHAIN_ACCOUNT,
                "-s",
                KEYCHAIN_SERVICE,
                "-w",
            ])
            .output()
        {
            if output.status.success() {
                let key = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if valid_hex_key(&key) {
                    return Ok(key);
                }
            }
        }
    }

    if let Some(path) = credentials_path() {
        if path.is_file() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|_| "读取微信凭据文件失败（请检查文件权限）".to_string())?;
            let value: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|_| "微信凭据文件格式不正确".to_string())?;
            if let Some(key) = value.get("passphrase_hex").and_then(|v| v.as_str()) {
                if valid_hex_key(key) {
                    return Ok(key.to_string());
                }
            }
        }
    }

    Err("未找到微信解密口令：请先在钥匙串添加 jycollector.wechat.key".to_string())
}

fn resolve_python() -> String {
    std::env::var("JY_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

fn last_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .last()
        .map(|line| line.trim_start_matches("采集失败：").to_string())
        .unwrap_or_else(|| "采集进程异常退出".to_string())
}

fn run_sidecar(sidecar_dir: &Path, request: &serde_json::Value) -> Result<WxExport, String> {
    let value = run_sidecar_json(sidecar_dir, request)?;
    serde_json::from_value(value)
        .map_err(|_| "采集结果解析失败（微信版本可能已变更）".to_string())
}

fn run_sidecar_json(
    sidecar_dir: &Path,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let script = sidecar_dir.join("wechat_export.py");
    if !script.is_file() {
        return Err(format!("采集脚本缺失：{}", script.display()));
    }

    let mut child = Command::new(resolve_python())
        .arg(&script)
        .current_dir(sidecar_dir)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "未找到 python3，无法启动微信采集进程".to_string())?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "无法写入采集进程".to_string())?;
        stdin
            .write_all(request.to_string().as_bytes())
            .map_err(|_| "无法写入采集进程".to_string())?;
    }

    let output = child
        .wait_with_output()
        .map_err(|_| "等待采集进程失败".to_string())?;

    if !output.status.success() {
        return Err(last_error_line(&String::from_utf8_lossy(&output.stderr)));
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|_| "采集结果解析失败（微信版本可能已变更）".to_string())
}

/// 解码管线版本。规则/解码逻辑变更时递增，下次同步会自动全量重读并覆盖旧行。
const PIPELINE_VERSION: &str = "5";

/// 解密 -> 归一化 -> 增量落库。返回本次同步的统计信息。
pub fn sync(
    conn: &mut rusqlite::Connection,
    sidecar_dir: &Path,
) -> Result<SyncSummaryDto, String> {
    if let Some(message) = SOURCE_UNAVAILABLE {
        return Err(message.to_string());
    }
    let started = std::time::Instant::now();
    let key_hex = load_key_hex()?;

    let stored_version = crate::db::setting_value(conn, "wechat_pipeline_version")
        .map_err(|e| e.to_string())?;
    let since = if stored_version.as_deref() == Some(PIPELINE_VERSION) {
        crate::db::wechat_cursors(conn).map_err(|e| e.to_string())?
    } else {
        Default::default()
    };

    let request = serde_json::json!({
        "keyHex": key_hex,
        "dbRoot": default_db_root().to_string_lossy(),
        "since": since,
    });
    let exported = run_sidecar(sidecar_dir, &request)?;

    let group_count = exported.groups.len() as i64;
    let message_count = exported.messages.len() as i64;
    let (_, added) = crate::db::apply_wechat_sync(
        conn,
        &exported.groups,
        &exported.messages,
        &exported.avatars,
        &exported.avatar_urls,
    )
    .map_err(|e| format!("写入本地库失败：{}", e))?;
    crate::db::save_setting(conn, "wechat_pipeline_version", PIPELINE_VERSION)
        .map_err(|e| e.to_string())?;

    Ok(SyncSummaryDto {
        account: exported.account,
        groups: group_count,
        messages: message_count,
        added,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

# JY Collector · Windows 使用说明

## 下载哪个

Release 里通常有三个文件：

- **免安装绿色版**：`JY-Collector-<版本>-windows-x64-portable.zip`
- **安装包**：`JY-Collector-<版本>-windows-x64-setup.exe`
- **校验值**：`SHA256SUMS.txt`

不确定电脑有没有安装权限时，先试绿色版。两者功能完全一样。

## 怎么启动

**绿色版**：解压到任意可写目录（例如桌面或文档），双击 `jy-collector.exe` 即可。
不要把它放进 `C:\Program Files`，也不要在压缩包里直接双击。

**安装包**：双击运行，按提示安装。安装范围是“仅当前用户”，装到
`%LOCALAPPDATA%`，正常情况下不会弹出 UAC 管理员授权。

数据统一保存在 `%APPDATA%\com.jy.collector`，不写程序目录，所以绿色版换目录
也不会丢数据。卸载安装版：设置 → 应用 → 已安装的应用 → jy-collector；
绿色版直接删除文件夹即可。

## 被 Windows 拦截怎么办

程序没有购买代码签名证书，Windows 会提示“未知发布者”，这是正常的。

1. 下载完成后，右键压缩包或 exe → 属性 → 勾选“解除锁定” → 确定。
2. 双击后出现“Windows 已保护你的电脑”：点“更多信息” → “仍要运行”。
3. 如果公司电脑的策略直接阻止运行，请联系 IT 放行，或改用绿色版。

## 缺少 WebView2 运行时

Win11 自带 WebView2；Win10 大多数已随 Edge 预装。如果启动时提示缺少
WebView2，请安装 Microsoft Edge WebView2 Runtime（Evergreen Bootstrapper，
x64）后再启动。没有管理员权限时，引导安装程序会装到当前用户。

## 读取本机微信聊天记录

1. 先登录电脑版微信，并保持微信在运行（进程名 `Weixin.exe`，微信 4.x）。
2. 打开应用，按首次设置向导依次完成“检测 → 授权 → 同步 → 选择会话”。
3. 如果提示“无法读取微信进程内存”，请关闭应用，右键 `jy-collector.exe` 选择
   “以管理员身份运行”，再重新检测。

说明：

- 只支持微信 4.x 的数据目录（`文档\xwechat_files`）；微信 3.x 暂不支持。
- 读取过程只读，不修改微信数据库，也不保存解密后的数据库到程序目录。
- 聊天图片目前可能显示为占位，文字消息、会话和头像可以正常采集。

## 核对下载是否完整

PowerShell 里执行：

```powershell
Get-FileHash .\JY-Collector-<版本>-windows-x64-portable.zip -Algorithm SHA256
```

把输出的哈希与 `SHA256SUMS.txt` 中对应文件名后的值对比，一致即可。

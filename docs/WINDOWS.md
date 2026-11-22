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

## Windows 上暂时不可用的功能

- **从本机微信读取并解密聊天记录**：尚未实现。采集页会显示一条简洁的状态说明，
  应用不会白屏或崩溃。
- 因为暂时没有数据源，会话列表和消息列表是空的；“查找”“数据”“记录”等界面可以
  正常打开，记录表、模板和台账导出在录入数据后即可使用。

## 核对下载是否完整

PowerShell 里执行：

```powershell
Get-FileHash .\JY-Collector-<版本>-windows-x64-portable.zip -Algorithm SHA256
```

把输出的哈希与 `SHA256SUMS.txt` 中对应文件名后的值对比，一致即可。

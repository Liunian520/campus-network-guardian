# 参与贡献

欢迎提交问题报告和 Pull Request。请先阅读 README，说明 Windows 版本、程序版本、预期行为和实际结果。界面问题可附脱敏截图；网络问题请只提供错误分类和必要的脱敏信息。

不要提交校园网账号、密码、Cookie、令牌、真实门户响应、个人配置或运行日志。需要复现协议问题时，使用虚构账号和本地模拟服务。涉及安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。

## 本地检查

使用 Windows 10/11 x64、Rust stable MSVC 工具链，以及 Python 3.12 或更新版本：

```powershell
cargo fmt --all -- --check
cargo test --locked --all-targets
python tools/check-ui-contrast.py --self-test
python tools/check-ui-contrast.py
```

默认测试使用模拟时钟、虚构凭据和回环地址服务，不访问 Windows 已保存的凭据或修改自启动。真实校园网络检查被忽略，不能加入 CI，也不应在未经同意时对正在使用的网络执行。

请为协议解析、恢复策略和安全边界的变化补充有意义的模拟测试。界面改动沿用 `src/ui_theme.rs` 的语义颜色，检查键盘操作、高对比度、不同 DPI 和短屏滚动；离屏预览方法见 README。保持原生、低资源占用的目标。

## 安装包与发布

安装 Inno Setup 6 后，可仅构建安装包：

```powershell
.\tools\build-installer.ps1 -IsccPath 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe'
```

这一步不运行安装器。维护者可手动运行 Windows release 工作流检查安装包和便携包；手动运行只上传工作流产物。发布时，Git 标签须为与 Cargo 版本一致的 `v主版本.次版本.修订版本`。标签工作流通过检查后才创建 GitHub Release，并附带安装包、便携 ZIP 和 SHA-256 校验文件。

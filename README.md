<div align="center">
  <img src="assets/icons/ssh.png" width="96" alt="Luna Remote 图标">
  <h1>Luna Remote</h1>
  <p>面向 macOS 和 Windows 的 SSH 与 SFTP 桌面客户端</p>
  <p>
    <a href="https://github.com/rainj2013/luna-remote/releases/latest"><img src="https://img.shields.io/github/v/release/rainj2013/luna-remote?style=flat-square" alt="最新版本"></a>
    <a href="https://github.com/rainj2013/luna-remote/actions/workflows/release.yml"><img src="https://github.com/rainj2013/luna-remote/actions/workflows/release.yml/badge.svg" alt="构建状态"></a>
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-5b8def?style=flat-square" alt="支持平台">
  </p>
  <p>
    <a href="https://github.com/rainj2013/luna-remote/releases/latest">下载最新版本</a>
    ·
    <a href="docs/DEVELOPMENT.md">开发文档</a>
  </p>
</div>

<div align="center"><a href="README.en.md">English</a> · 简体中文</div>

Luna Remote 把日常远程工作放进一个窗口。连接服务器后，可以随时在终端和 SFTP 之间切换，也能直接管理端口转发、连接配置和多个并行会话。

![Luna Remote 终端界面](docs/images/terminal.png)

## SSH 终端

- 支持密码、键盘交互、私钥和 SSH Agent 认证
- 提供 Host Key 校验、会话保活和一级跳板机
- 同一台服务器可以打开多个独立会话
- 连接支持分组、排序、折叠和 SSH Config 导入
- 字体、字号、配色、透明度和背景图均可调整
- 支持深色、浅色和跟随系统的界面主题

## SFTP 与目录部署

本地目录和远程目录采用双栏布局。文件和目录可以直接上传、下载、预览或拖放，传输任务会进入队列，失败后可以重试。

目录部署会先比较本地与远程内容，再批量上传有变化的文件。常用路径可以收藏，遇到同名文件时也可以选择覆盖、跳过或自动重命名。

![Luna Remote SFTP 文件传输界面](docs/images/sftp.png)

## 端口转发

Luna Remote 支持本地转发、远程转发和 SOCKS5 动态转发。转发规则跟随连接保存，需要时可以直接启用，不必重复填写主机和端口。

## AI 命令助手

用自然语言描述任务后，AI 命令助手可以生成 Linux Shell、PowerShell、CMD 或 macOS 命令，并说明命令用途、执行前提和潜在风险。

生成结果可以复制或填入当前终端，执行前仍由用户确认。附带终端上下文时，最近的输出会先经过常见凭据和个人信息脱敏。AI 功能支持 OpenAI 兼容服务，不配置也不会影响 SSH 和 SFTP。

![Luna Remote AI 命令助手](docs/images/ai-command.png)

## 本地数据与个性化

- 凭据保存在 macOS Keychain 或 Windows Credential Manager
- 连接配置支持导出、备份和恢复
- 文件传输提供历史记录和失败重试
- 应用图标、界面主题和终端外观均可更换

## 下载

安装包可以从 [Releases](https://github.com/rainj2013/luna-remote/releases/latest) 获取。

| 平台 | 安装包 | 说明 |
| --- | --- | --- |
| macOS Apple Silicon | `Luna-Remote_macos-arm64.dmg` | 适用于 M 系列芯片 |
| macOS Intel | `Luna-Remote_macos-x64.dmg` | 适用于 Intel Mac |
| Windows x64 | `Luna-Remote_windows-x64-online-setup.exe` | 缺少 WebView2 时联网下载 |
| Windows x64 | `Luna-Remote_windows-x64-offline-setup.exe` | 内置 WebView2，适合离线安装 |

当前安装包没有开发者签名。macOS 首次运行时需要在“隐私与安全性”中确认，Windows 可能显示未知发布者或 SmartScreen 提示。

## 开发

项目结构、环境搭建、检查命令和打包发布说明见 [开发文档](docs/DEVELOPMENT.md)。

## 许可证

Luna Remote 基于 [MIT License](LICENSE) 发布。项目使用的第三方组件和字体保留其各自的许可证。

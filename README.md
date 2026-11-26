# ❄ Snow

Windows 桌面下雪小程序。雪花从屏幕上方飘落，在所有窗口上方显示，但不挡鼠标，可以照常点击、打字。

用 Rust 编写，通过 DirectComposition + Direct2D 在显卡上绘制，exe 只有几百 KB，不用安装。

## 功能（v0.1）

- 全屏下雪，支持多显示器和高 DPI
- 降雪量：小雪 / 中雪 / 大雪 / 暴风雪（带阵风）
- 风：无风 / 微风 / 大风，可选向左或向右吹
- 雪花大小：小 / 中 / 大
- `Ctrl+Alt+S` 暂停 / 继续
- 开机自动启动
- 设置自动保存在 `%APPDATA%\Snow\config.txt`

所有操作都在屏幕右下角托盘的 ❄ 图标里（右键或左键单击）。如果看不到图标，点任务栏上的 **^**。

## 计划

- [ ] 雪落在窗口顶部和任务栏上积成雪堆
- [ ] 用鼠标扫雪
- [ ] 彩灯
- [ ] 全屏游戏 / 视频时自动暂停，用电池时省电
- [ ] 扫雪车、雪人、企鹅、圣诞老人

## 编译

需要 Rust。在 Windows 上：

```bash
cargo build --release --target x86_64-pc-windows-msvc
```

在 Linux 上交叉编译（需要 `mingw-w64`）：

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release
```

生成的程序在 `target/x86_64-pc-windows-gnu/release/snow.exe`。

模拟部分的单元测试可以在任何系统上运行：

```bash
cargo test --target x86_64-unknown-linux-gnu
```

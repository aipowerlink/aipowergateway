# assets

## icon（应用图标：算力 + 共享 + 安全）

| 文件 | 用途 |
|------|------|
| `icon-source.png` | 用户提供的设计图（2048×2048，白底已转透明） |
| `icon-source-alpha.png` | 白底透明化后的源图 |
| `icon.ico` | Windows exe 图标（7 尺寸合一：16/24/32/48/64/128/256） |
| `png/icon-16..512.png` | 各尺寸 PNG（托盘 32、favicon 16/32/180、Linux 512） |

### 设计语义

- **算力**：环形节点设计，青→绿渐变（#17B1CD → #65D986）
- **共享**：主体环形覆盖 75% 宽，四周分布（参与者互连）
- **安全**：环形结构（可信边界），青色主调（网络安全）

### 再生成

```bash
# SVG → PNG（sharp，在 web/ 目录）
npm install sharp
node -e "const s=require('sharp');const fs=require('fs');[16,24,32,48,64,128,256,512].forEach(async n=>{await s(fs.readFileSync('../assets/icon.svg')).resize(n,n).png().toFile('../assets/png/icon-'+n+'.png')})"

# ICO 由 icon.ico 已生成（多尺寸 PNG 打包）
```

### 使用位置

- 托盘图标：`crates/lan-tray/src/tray.rs`（include_bytes! 嵌入 icon-32.png）
- Windows exe：`crates/cli/resources/app.rc`（IDI_ICON1 → icon.ico）
- 网页 favicon：`web/public/favicon-*` + apple-touch-icon
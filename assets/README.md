# assets

## icon（应用图标：算力 + 共享 + 安全）

| 文件 | 用途 |
|------|------|
| `icon.svg` | 矢量源（单一事实源，512×512 设计稿） |
| `icon.ico` | Windows exe 图标（7 尺寸合一：16/24/32/48/64/128/256） |
| `png/icon-16..512.png` | 各尺寸 PNG（托盘 32、favicon 16/32/180、Linux 512） |

### 设计语义

- **算力（Compute）**：中心盾牌内闪电（能量/计算核心），蓝→紫渐变底
- **共享（Share）**：四周分布式链路 + 节点圆点（参与者互连）
- **安全（Security）**：中心盾牌 + 对勾（安全确认），绿色盾面

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
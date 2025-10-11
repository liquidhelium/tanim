# 安装和运行指南

## 前置要求

在开始之前，请确保安装了以下工具：

1. **Node.js** (版本 18 或更高)
   - 下载地址: https://nodejs.org/
   - 安装后会自动包含 npm

2. **Visual Studio Code**
   - 下载地址: https://code.visualstudio.com/

## 安装步骤

### 1. 安装Node.js
如果还没有安装Node.js，请先安装它：

1. 访问 https://nodejs.org/
2. 下载LTS版本
3. 运行安装程序
4. 安装完成后，重启命令行或PowerShell

验证安装：
```powershell
node --version
npm --version
```

### 2. 安装项目依赖

在项目目录中运行：

```powershell
cd c:\code\tanim-vscode
npm install
```

这会安装以下依赖：
- `@types/vscode` - VS Code API类型定义
- `@types/node` - Node.js类型定义
- `typescript` - TypeScript编译器
- `eslint` - 代码检查工具

### 3. 编译项目

```powershell
npm run compile
```

或者在开发时使用监视模式：

```powershell
npm run watch
```

### 4. 运行扩展

有两种方式运行扩展：

#### 方式一：使用VS Code调试
1. 在VS Code中打开项目文件夹
2. 按 `F5` 或点击"运行和调试"
3. 选择"Run Extension"
4. 会打开一个新的VS Code窗口，扩展已加载

#### 方式二：打包安装
```powershell
# 安装vsce打包工具
npm install -g @vscode/vsce

# 打包扩展
npx vsce package

# 安装生成的.vsix文件
# 在VS Code中: 扩展 -> ... -> 从VSIX安装
```

## 使用扩展

1. 在新窗口中创建或打开一个 `.typ` 文件
2. 右下角状态栏会显示 "$(symbol-numeric) Animation: 0"
3. 点击状态栏项目打开滑块控制面板
4. 在面板中调整滑块或输入框来设置动画参数

## 配置说明

### 扩展配置项

在 VS Code 设置中 (File -> Preferences -> Settings)，搜索 "tanim-slider"：

- **Min Value** (`tanim-slider.minValue`)
  - 滑块最小值
  - 默认: 0

- **Max Value** (`tanim-slider.maxValue`)
  - 滑块最大值
  - 默认: 100

- **Default Value** (`tanim-slider.defaultValue`)
  - 默认初始值
  - 默认: 0

- **Target Config Key** (`tanim-slider.targetConfigKey`)
  - 要拦截并代理的配置键
  - 默认: "tinymist.preview.cursorIndicator"
  - 示例: "extension.config.key"

### 如何工作

扩展会：
1. 监听当前打开的文件，只在Typst文件时显示
2. 在状态栏显示当前参数值
3. 拦截目标配置的读取，用滑块的值替代
4. 保存最后使用的值，下次启动时恢复

## 目录结构

```
tanim-vscode/
├── .vscode/              # VS Code配置
│   ├── launch.json       # 调试配置
│   ├── tasks.json        # 任务配置
│   └── settings.json     # 工作区设置
├── src/                  # 源代码
│   ├── extension.ts      # 扩展入口
│   ├── sliderPanel.ts    # 滑块面板
│   └── configInterceptor.ts  # 配置拦截器
├── out/                  # 编译输出(自动生成)
├── package.json          # 项目配置
├── tsconfig.json         # TypeScript配置
├── .eslintrc.json        # ESLint配置
├── .gitignore            # Git忽略文件
├── README.md             # 说明文档
└── INSTALL.md            # 本文件
```

## 故障排除

### 编译错误
如果遇到TypeScript编译错误：
```powershell
# 清理并重新安装
rm -r node_modules, out
npm install
npm run compile
```

### 扩展未激活
- 确保打开的是 `.typ` 文件
- 检查VS Code输出面板中的错误信息
- 按 `Ctrl+Shift+P` 输入 "Developer: Reload Window"

### 状态栏不显示
- 确认当前文件语言模式是 "Typst"
- 在文件底部右侧点击语言模式，选择 "Typst"

## 开发

### 修改代码后
1. 保存文件
2. 如果运行了 `npm run watch`，会自动重新编译
3. 在扩展开发主机窗口按 `Ctrl+R` 重新加载扩展

### 调试
- 在源代码中设置断点
- 按 `F5` 启动调试
- 在扩展开发主机窗口中触发功能
- 断点会在主窗口中命中

## 更多信息

- [VS Code扩展开发文档](https://code.visualstudio.com/api)
- [TypeScript文档](https://www.typescriptlang.org/docs/)
- [Typst语言](https://typst.app/)

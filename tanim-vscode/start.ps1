# Typst Animation Slider - 快速启动脚本

Write-Host "=====================================" -ForegroundColor Cyan
Write-Host "  Typst Animation Slider Extension  " -ForegroundColor Cyan
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host ""

# 检查Node.js和npm
Write-Host "检查环境..." -ForegroundColor Yellow

$nodeVersion = $null
$npmVersion = $null

try {
    $nodeVersion = node --version 2>$null
    $npmVersion = npm --version 2>$null
} catch {
    # 忽略错误
}

if (-not $nodeVersion) {
    Write-Host "❌ 未找到 Node.js" -ForegroundColor Red
    Write-Host ""
    Write-Host "请先安装 Node.js:" -ForegroundColor Yellow
    Write-Host "1. 访问: https://nodejs.org/" -ForegroundColor White
    Write-Host "2. 下载并安装 LTS 版本" -ForegroundColor White
    Write-Host "3. 重启 PowerShell" -ForegroundColor White
    Write-Host "4. 再次运行此脚本" -ForegroundColor White
    Write-Host ""
    Read-Host "按 Enter 键退出"
    exit 1
}

Write-Host "✅ Node.js: $nodeVersion" -ForegroundColor Green
Write-Host "✅ npm: $npmVersion" -ForegroundColor Green
Write-Host ""

# 检查是否已安装依赖
if (-not (Test-Path "node_modules")) {
    Write-Host "正在安装依赖..." -ForegroundColor Yellow
    npm install
    if ($LASTEXITCODE -ne 0) {
        Write-Host "❌ 依赖安装失败" -ForegroundColor Red
        Read-Host "按 Enter 键退出"
        exit 1
    }
    Write-Host "✅ 依赖安装成功" -ForegroundColor Green
    Write-Host ""
} else {
    Write-Host "✅ 依赖已安装" -ForegroundColor Green
    Write-Host ""
}

# 编译项目
Write-Host "正在编译项目..." -ForegroundColor Yellow
npm run compile
if ($LASTEXITCODE -ne 0) {
    Write-Host "❌ 编译失败" -ForegroundColor Red
    Read-Host "按 Enter 键退出"
    exit 1
}
Write-Host "✅ 编译成功" -ForegroundColor Green
Write-Host ""

# 显示菜单
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host "  请选择操作:" -ForegroundColor Cyan
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "1. 在 VS Code 中打开项目（按F5调试）" -ForegroundColor White
Write-Host "2. 启动监视模式（自动编译）" -ForegroundColor White
Write-Host "3. 重新编译" -ForegroundColor White
Write-Host "4. 打包扩展（.vsix）" -ForegroundColor White
Write-Host "5. 清理并重新安装" -ForegroundColor White
Write-Host "6. 退出" -ForegroundColor White
Write-Host ""

$choice = Read-Host "请输入选项 (1-6)"

switch ($choice) {
    "1" {
        Write-Host ""
        Write-Host "正在打开 VS Code..." -ForegroundColor Yellow
        Write-Host "打开后按 F5 键启动扩展调试" -ForegroundColor Cyan
        code .
    }
    "2" {
        Write-Host ""
        Write-Host "启动监视模式..." -ForegroundColor Yellow
        Write-Host "代码更改会自动编译" -ForegroundColor Cyan
        Write-Host "按 Ctrl+C 停止" -ForegroundColor Cyan
        Write-Host ""
        npm run watch
    }
    "3" {
        Write-Host ""
        Write-Host "正在编译..." -ForegroundColor Yellow
        npm run compile
        Write-Host "✅ 编译完成" -ForegroundColor Green
    }
    "4" {
        Write-Host ""
        Write-Host "正在打包扩展..." -ForegroundColor Yellow
        
        # 检查是否安装了vsce
        $vsceVersion = $null
        try {
            $vsceVersion = npx vsce --version 2>$null
        } catch {
            # 忽略
        }
        
        if (-not $vsceVersion) {
            Write-Host "安装打包工具..." -ForegroundColor Yellow
            npm install -g @vscode/vsce
        }
        
        npx vsce package
        
        if ($LASTEXITCODE -eq 0) {
            Write-Host "✅ 打包成功！" -ForegroundColor Green
            Write-Host ""
            Write-Host "生成的 .vsix 文件可以直接安装到 VS Code" -ForegroundColor Cyan
            Write-Host "在 VS Code 中: 扩展 -> ... -> 从VSIX安装" -ForegroundColor Cyan
        } else {
            Write-Host "❌ 打包失败" -ForegroundColor Red
        }
    }
    "5" {
        Write-Host ""
        Write-Host "正在清理..." -ForegroundColor Yellow
        
        if (Test-Path "node_modules") {
            Remove-Item -Recurse -Force "node_modules"
            Write-Host "✅ 已删除 node_modules" -ForegroundColor Green
        }
        
        if (Test-Path "out") {
            Remove-Item -Recurse -Force "out"
            Write-Host "✅ 已删除 out" -ForegroundColor Green
        }
        
        Write-Host ""
        Write-Host "正在重新安装..." -ForegroundColor Yellow
        npm install
        
        Write-Host ""
        Write-Host "正在编译..." -ForegroundColor Yellow
        npm run compile
        
        Write-Host "✅ 完成！" -ForegroundColor Green
    }
    "6" {
        Write-Host "再见！" -ForegroundColor Cyan
        exit 0
    }
    default {
        Write-Host "无效的选项" -ForegroundColor Red
    }
}

Write-Host ""
Read-Host "按 Enter 键退出"

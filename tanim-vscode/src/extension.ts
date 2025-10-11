import * as vscode from 'vscode';
import { ConfigInterceptor } from './configInterceptor';
import { SliderViewProvider } from './sliderViewProvider';

let configInterceptor: ConfigInterceptor | undefined;
let statusBarItem: vscode.StatusBarItem | undefined;
let sliderViewProvider: SliderViewProvider | undefined;

export function activate(context: vscode.ExtensionContext) {
    console.log('Typst Animation Slider extension is now active');

    // 创建配置拦截器
    configInterceptor = new ConfigInterceptor(context);

    // 注册 Webview View Provider（侧边栏）
    sliderViewProvider = new SliderViewProvider(context.extensionUri, configInterceptor);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(
            SliderViewProvider.viewType,
            sliderViewProvider
        )
    );

    // 创建状态栏项
    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBarItem.command = 'tanim-slider.focusView';
    updateStatusBarItem(configInterceptor.getCurrentValue());
    context.subscriptions.push(statusBarItem);

    // 监听值变化以更新状态栏
    configInterceptor.onValueChanged((value) => {
        updateStatusBarItem(value);
    });

    // 注册命令 - 聚焦到侧边栏视图
    const focusViewCommand = vscode.commands.registerCommand(
        'tanim-slider.focusView',
        () => {
            vscode.commands.executeCommand('tanim-slider.view.focus');
        }
    );

    context.subscriptions.push(focusViewCommand);

    // 监听活动编辑器变化
    context.subscriptions.push(
        vscode.window.onDidChangeActiveTextEditor((editor) => {
            updateStatusBarVisibility(editor);
        })
    );

    // 初始化时检查当前编辑器
    updateStatusBarVisibility(vscode.window.activeTextEditor);

    // 监听配置变化
    context.subscriptions.push(
        vscode.workspace.onDidChangeConfiguration((e) => {
            if (e.affectsConfiguration('tanim-slider')) {
                configInterceptor?.updateConfig();
                sliderViewProvider?.updateRange();
            }
        })
    );
}

function updateStatusBarItem(value: number) {
    if (statusBarItem) {
        statusBarItem.text = `$(symbol-numeric) t=${value}`;
        statusBarItem.tooltip = 'Click to open Typst Animation Slider';
    }
}

function updateStatusBarVisibility(editor: vscode.TextEditor | undefined) {
    if (editor && editor.document.languageId === 'typst') {
        statusBarItem?.show();
    } else {
        statusBarItem?.hide();
    }
}

export function deactivate() {
    // 清理资源
    if (configInterceptor) {
        configInterceptor.dispose();
    }
}

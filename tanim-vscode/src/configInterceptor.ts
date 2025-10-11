import * as vscode from 'vscode';

/**
 * ConfigInterceptor - 直接修改当前 Typst 文件中的代码
 */
export class ConfigInterceptor {
    private _context: vscode.ExtensionContext;
    private _overrideValue: number;
    private _disposables: vscode.Disposable[] = [];
    private _onValueChanged?: (value: number) => void;

    constructor(context: vscode.ExtensionContext) {
        this._context = context;

        const config = vscode.workspace.getConfiguration('tanim-slider');
        this._overrideValue = config.get<number>('defaultValue', 0);

        // 从存储中恢复上次的值
        const savedValue = context.globalState.get<number>('animationValue');
        if (savedValue !== undefined) {
            this._overrideValue = savedValue;
        } else {
            // 尝试从当前文件中读取
            this._overrideValue = this.readCurrentValue();
        }
    }

    /**
     * 从当前活动编辑器中读取匹配模式的值
     */
    private readCurrentValue(): number {
        try {
            const editor = vscode.window.activeTextEditor;
            if (editor && editor.document.languageId === 'typst') {
                const text = editor.document.getText();
                const config = vscode.workspace.getConfiguration('tanim-slider');
                const patterns = config.get<Array<{ pattern: string, replacement: string }>>('searchPatterns', []);

                // 尝试所有模式
                for (const patternConfig of patterns) {
                    try {
                        const regex = new RegExp(patternConfig.pattern);
                        const match = text.match(regex);
                        if (match && match[1]) {
                            return parseFloat(match[1]);
                        }
                    } catch (error) {
                        console.error(`Invalid regex pattern: ${patternConfig.pattern}`, error);
                    }
                }
            }
        } catch (error) {
            console.error('Failed to read current value:', error);
        }

        // 返回默认值
        const config = vscode.workspace.getConfiguration('tanim-slider');
        return config.get<number>('defaultValue', 0);
    }

    /**
     * 更新所有打开的 Typst 文件中匹配模式的值
     * 不保存文件,只修改编辑器内容
     */
    private async updateTypstFile(): Promise<boolean> {
        try {
            // 获取所有打开的文本编辑器
            const editors = vscode.window.visibleTextEditors.filter(
                editor => editor.document.languageId === 'typst'
            );

            if (editors.length === 0) {
                console.log('No visible Typst editors found');
                return false;
            }

            // 获取配置的模式
            const config = vscode.workspace.getConfiguration('tanim-slider');
            const patterns = config.get<Array<{ pattern: string, replacement: string }>>('searchPatterns', []);

            if (patterns.length === 0) {
                console.warn('No search patterns configured');
                return false;
            }

            let updated = false;

            for (const editor of editors) {
                const document = editor.document;
                const text = document.getText();

                // 使用 TextEditorEdit 来修改文件（不触发保存）
                await editor.edit(editBuilder => {
                    // 尝试所有配置的模式
                    for (const patternConfig of patterns) {
                        try {
                            const regex = new RegExp(patternConfig.pattern, 'g');
                            const matches = [...text.matchAll(regex)];

                            if (matches.length === 0) {
                                continue;
                            }

                            for (const match of matches) {
                                if (match.index === undefined) { continue; }

                                const startPos = document.positionAt(match.index);
                                const endPos = document.positionAt(match.index + match[0].length);
                                const range = new vscode.Range(startPos, endPos);

                                // 使用 replacement 模板，将 ${value} 替换为实际值
                                const newText = patternConfig.replacement.replace(/\$\{value\}/g, String(this._overrideValue));

                                editBuilder.replace(range, newText);
                            }

                            console.log(`Updated Typst editor with pattern: ${patternConfig.pattern}, t=${this._overrideValue}`);
                            updated = true;
                        } catch (error) {
                            console.error(`Invalid regex pattern: ${patternConfig.pattern}`, error);
                        }
                    }
                });
            }

            return updated;
        } catch (error) {
            console.error('Failed to update Typst file:', error);
            return false;
        }
    }

    /**
     * 设置覆盖值
     */
    public async setOverrideValue(value: number) {
        this._overrideValue = value;

        // 保存到持久化存储
        await this._context.globalState.update('animationValue', value);

        // 更新 Typst 文件
        await this.updateTypstFile();

        // 触发回调
        if (this._onValueChanged) {
            this._onValueChanged(value);
        }
    }

    /**
     * 获取当前覆盖值
     */
    public getCurrentValue(): number {
        return this._overrideValue;
    }

    /**
     * 设置值变化回调
     */
    public onValueChanged(callback: (value: number) => void) {
        this._onValueChanged = callback;
    }

    /**
     * 更新配置（当扩展配置变化时调用）
     */
    public updateConfig() {
        // 预留接口，暂时不需要实现
    }

    /**
     * 释放资源
     */
    public dispose() {
        while (this._disposables.length) {
            const disposable = this._disposables.pop();
            if (disposable) {
                disposable.dispose();
            }
        }
    }
}

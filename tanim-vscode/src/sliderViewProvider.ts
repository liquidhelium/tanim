import * as vscode from 'vscode';
import { ConfigInterceptor } from './configInterceptor';

export class SliderViewProvider implements vscode.WebviewViewProvider {
    public static readonly viewType = 'tanim-slider.view';
    private _view?: vscode.WebviewView;
    private _configInterceptor: ConfigInterceptor;

    constructor(
        private readonly _extensionUri: vscode.Uri,
        configInterceptor: ConfigInterceptor
    ) {
        this._configInterceptor = configInterceptor;
    }

    public resolveWebviewView(
        webviewView: vscode.WebviewView,
        context: vscode.WebviewViewResolveContext,
        _token: vscode.CancellationToken,
    ) {
        this._view = webviewView;

        webviewView.webview.options = {
            enableScripts: true,
            localResourceRoots: [this._extensionUri]
        };

        webviewView.webview.html = this._getHtmlForWebview(webviewView.webview);

        webviewView.webview.onDidReceiveMessage(data => {
            switch (data.command) {
                case 'valueChanged':
                    this._configInterceptor.setOverrideValue(data.value);
                    break;
            }
        });
    }

    public updateRange() {
        if (this._view) {
            const config = vscode.workspace.getConfiguration('tanim-slider');
            const min = config.get<number>('minValue', 0);
            const max = config.get<number>('maxValue', 100);
            const current = this._configInterceptor.getCurrentValue();

            this._view.webview.postMessage({
                command: 'updateRange',
                min: min,
                max: max,
                value: current
            });
        }
    }

    private _getHtmlForWebview(webview: vscode.Webview) {
        const config = vscode.workspace.getConfiguration('tanim-slider');
        const min = config.get<number>('minValue', 0);
        const max = config.get<number>('maxValue', 100);
        const defaultValue = this._configInterceptor.getCurrentValue();

        return `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Animation Slider</title>
    <style>
        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }

        body {
            font-family: var(--vscode-font-family);
            padding: 15px;
            color: var(--vscode-foreground);
            background-color: var(--vscode-sideBar-background);
        }

        .container {
            display: flex;
            flex-direction: column;
            gap: 15px;
        }

        .value-display {
            font-size: 48px;
            font-weight: bold;
            text-align: center;
            color: var(--vscode-textLink-foreground);
            margin: 10px 0;
        }

        .slider-section {
            display: flex;
            flex-direction: column;
            gap: 10px;
        }

        .range-inputs {
            display: flex;
            justify-content: space-between;
            gap: 10px;
        }

        .input-group {
            flex: 1;
            display: flex;
            flex-direction: column;
            gap: 4px;
        }

        label {
            font-size: 11px;
            color: var(--vscode-descriptionForeground);
            text-transform: uppercase;
        }

        .input-box {
            width: 100%;
            padding: 6px;
            font-size: 13px;
            background-color: var(--vscode-input-background);
            color: var(--vscode-input-foreground);
            border: 1px solid var(--vscode-input-border);
            border-radius: 3px;
            text-align: center;
        }

        .input-box:focus {
            outline: 1px solid var(--vscode-focusBorder);
        }

        .slider {
            width: 100%;
            height: 8px;
            background: var(--vscode-progressBar-background);
            outline: none;
            -webkit-appearance: none;
            appearance: none;
            border-radius: 4px;
            cursor: pointer;
        }

        .slider::-webkit-slider-thumb {
            -webkit-appearance: none;
            appearance: none;
            width: 20px;
            height: 20px;
            background: var(--vscode-button-background);
            cursor: pointer;
            border-radius: 50%;
            border: 2px solid var(--vscode-sideBar-background);
            box-shadow: 0 2px 4px rgba(0,0,0,0.2);
        }

        .slider::-moz-range-thumb {
            width: 20px;
            height: 20px;
            background: var(--vscode-button-background);
            cursor: pointer;
            border-radius: 50%;
            border: 2px solid var(--vscode-sideBar-background);
            box-shadow: 0 2px 4px rgba(0,0,0,0.2);
        }

        .slider:hover::-webkit-slider-thumb {
            background: var(--vscode-button-hoverBackground);
        }

        .slider:hover::-moz-range-thumb {
            background: var(--vscode-button-hoverBackground);
        }

        .info {
            font-size: 11px;
            color: var(--vscode-descriptionForeground);
            text-align: center;
            padding: 8px;
            background-color: var(--vscode-sideBar-dropBackground);
            border-radius: 4px;
        }

        .playback-controls {
            display: flex;
            align-items: center;
            gap: 10px;
            margin-top: 5px;
        }

        .fps-input-wrapper {
            flex: 1;
            display: flex;
            align-items: center;
            gap: 8px;
        }

        .fps-input {
            width: 60px;
            padding: 6px;
            font-size: 13px;
            background-color: var(--vscode-input-background);
            color: var(--vscode-input-foreground);
            border: 1px solid var(--vscode-input-border);
            border-radius: 3px;
            text-align: center;
        }

        .fps-input:focus {
            outline: 1px solid var(--vscode-focusBorder);
        }

        .fps-label {
            font-size: 13px;
            color: var(--vscode-foreground);
        }

        .play-button {
            padding: 8px 12px;
            background-color: var(--vscode-button-background);
            color: var(--vscode-button-foreground);
            border: none;
            border-radius: 3px;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
            min-width: 80px;
            font-size: 13px;
            transition: background-color 0.1s;
        }

        .play-button:hover {
            background-color: var(--vscode-button-hoverBackground);
        }

        .play-button.playing {
            background-color: var(--vscode-inputOption-activeBackground);
        }

        .play-button.playing:hover {
            background-color: var(--vscode-inputOption-activeBorder);
        }

        .play-icon {
            margin-right: 6px;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="value-display" id="displayValue">${defaultValue}</div>

        <div class="slider-section">
            <input type="range" id="slider" class="slider" 
                   min="${min}" max="${max}" value="${defaultValue}" step="1">
            
            <div class="range-inputs">
                <div class="input-group">
                    <label for="minInput">Min</label>
                    <input type="number" id="minInput" class="input-box" value="${min}">
                </div>
                <div class="input-group">
                    <label for="maxInput">Max</label>
                    <input type="number" id="maxInput" class="input-box" value="${max}">
                </div>
            </div>
        </div>

        <div class="playback-controls">
            <div class="fps-input-wrapper">
                <input type="number" id="fpsInput" class="fps-input" value="30" min="1" max="120">
                <span class="fps-label">fps</span>
            </div>
            <button id="playButton" class="play-button">
                <span class="play-icon">▶</span>
                <span id="playButtonText">Play</span>
            </button>
        </div>

        <div class="info" id="rangeDisplay">Range: ${min} - ${max}</div>
    </div>

    <script>
        const vscode = acquireVsCodeApi();
        const slider = document.getElementById('slider');
        const minInput = document.getElementById('minInput');
        const maxInput = document.getElementById('maxInput');
        const displayValue = document.getElementById('displayValue');
        const rangeDisplay = document.getElementById('rangeDisplay');
        const fpsInput = document.getElementById('fpsInput');
        const playButton = document.getElementById('playButton');
        const playButtonText = document.getElementById('playButtonText');
        const playIcon = playButton.querySelector('.play-icon');

        let isPlaying = false;
        let playbackInterval = null;

        function updateValue(value) {
            displayValue.textContent = value;
            vscode.postMessage({
                command: 'valueChanged',
                value: parseFloat(value)
            });
        }

        function updateRange() {
            const min = parseFloat(minInput.value);
            const max = parseFloat(maxInput.value);
            
            if (min >= max) {
                return;
            }

            slider.min = min;
            slider.max = max;
            
            if (parseFloat(slider.value) < min) {
                slider.value = min;
                updateValue(min);
            } else if (parseFloat(slider.value) > max) {
                slider.value = max;
                updateValue(max);
            }
            
            rangeDisplay.textContent = 'Range: ' + min + ' - ' + max;
        }

        function startPlayback() {
            if (playbackInterval) {
                clearInterval(playbackInterval);
            }

            const fps = parseFloat(fpsInput.value) || 30;
            const interval = 1000 / fps;
            const min = parseFloat(slider.min);
            const max = parseFloat(slider.max);

            playbackInterval = setInterval(() => {
                let currentValue = parseFloat(slider.value);
                currentValue += 1;

                if (currentValue > max) {
                    currentValue = min;
                }

                slider.value = currentValue;
                updateValue(currentValue);
            }, interval);
        }

        function stopPlayback() {
            if (playbackInterval) {
                clearInterval(playbackInterval);
                playbackInterval = null;
            }
        }

        function togglePlayback() {
            isPlaying = !isPlaying;

            if (isPlaying) {
                playButton.classList.add('playing');
                playIcon.textContent = '⏸';
                playButtonText.textContent = 'Pause';
                startPlayback();
            } else {
                playButton.classList.remove('playing');
                playIcon.textContent = '▶';
                playButtonText.textContent = 'Play';
                stopPlayback();
            }
        }

        slider.addEventListener('input', (e) => {
            updateValue(e.target.value);
        });

        minInput.addEventListener('change', updateRange);
        maxInput.addEventListener('change', updateRange);

        playButton.addEventListener('click', togglePlayback);

        fpsInput.addEventListener('change', () => {
            if (isPlaying) {
                stopPlayback();
                startPlayback();
            }
        });

        window.addEventListener('message', event => {
            const message = event.data;
            switch (message.command) {
                case 'updateRange':
                    minInput.value = message.min;
                    maxInput.value = message.max;
                    slider.min = message.min;
                    slider.max = message.max;
                    slider.value = message.value;
                    displayValue.textContent = message.value;
                    rangeDisplay.textContent = 'Range: ' + message.min + ' - ' + message.max;
                    break;
            }
        });
    </script>
</body>
</html>`;
    }
}

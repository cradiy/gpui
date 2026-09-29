import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import test from 'node:test';

const run = promisify(execFile);
const page = `<!doctype html><meta charset="utf-8"><body><script type="module">
import {configureImeInput, positionImeInput} from '/ime.js';
try {
    const close = (actual, expected, name) => {
        if (Math.abs(actual - expected) > 0.1) throw Error(name + ': ' + actual + ' != ' + expected);
    };
    document.body.style.height = '2400px';
    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:absolute;left:80px;top:100px;width:640px;height:360px;';
    document.body.append(canvas);
    const input = document.createElement('input');
    input.style.cssText = 'border:12px solid red;padding:20px;margin:30px;min-height:60px;';
    configureImeInput(input);
    document.body.append(input);
    input.value = '正在输入';
    input.focus({preventScroll:true});
    input.setSelectionRange(1, 3);
    const update = () => positionImeInput(canvas, input, 120, 50, 20, 640, 360);
    const expect = (x, y, height) => {
        update();
        const rect = input.getBoundingClientRect();
        close(rect.left, x, 'left');
        close(rect.top, y, 'top');
        close(rect.height, height, 'height');
        close(rect.width, 1, 'width');
    };
    expect(200, 150, 20);
    canvas.width = Math.round(640 * devicePixelRatio);
    canvas.height = Math.round(360 * devicePixelRatio);
    expect(200, 150, 20);
    canvas.style.border = '8px solid black';
    canvas.style.padding = '8px';
    expect(216, 166, 20);
    canvas.style.boxSizing = 'border-box';
    canvas.style.width = '672px';
    canvas.style.height = '392px';
    expect(216, 166, 20);
    canvas.style.transformOrigin = '0 0';
    canvas.style.transform = 'scale(1.25, 0.8)';
    expect(250, 152.8, 16);
    canvas.style.transform = '';
    canvas.style.zoom = '1.25';
    const zoomed = canvas.getBoundingClientRect();
    expect(zoomed.left + 170, zoomed.top + 82.5, 25);
    canvas.style.zoom = '';
    canvas.style.top = '800px';
    window.scrollTo(0, 300);
    expect(216, 866 - window.scrollY, 20);
    const before = input.style.cssText;
    positionImeInput(canvas, input, 0, 0, 20, 0, 0);
    if (input.style.cssText !== before) throw Error('zero size changed anchor');
    if (document.activeElement !== input || input.value !== '正在输入' || input.selectionStart !== 1 || input.selectionEnd !== 3)
        throw Error('positioning changed the input session');
    if (getComputedStyle(input).pointerEvents !== 'none') throw Error('input intercepts pointer events');
    document.body.dataset.result = 'passed';
} catch (error) {
    document.body.dataset.result = String(error);
}
</script>`;

test('IME anchor follows canvas geometry without altering the input session', async () => {
    const source = await readFile(new URL('./ime.js', import.meta.url));
    const server = createServer((req, res) => {
        res.setHeader('Content-Type', req.url === '/ime.js' ? 'text/javascript' : 'text/html');
        res.end(req.url === '/ime.js' ? source : page);
    });
    await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', resolve);
    });
    try {
        for (const dpr of [1, 1.5, 2]) {
            const profile = await mkdtemp(join(tmpdir(), 'gpui-ime-'));
            try {
                const {stdout} = await run(process.env.CHROME || 'google-chrome', [
                    '--headless=new', '--no-sandbox', '--disable-gpu', '--no-first-run',
                    '--disable-background-networking', `--user-data-dir=${profile}`,
                    `--force-device-scale-factor=${dpr}`, '--virtual-time-budget=1000',
                    '--dump-dom', `http://127.0.0.1:${server.address().port}`,
                ], {timeout: 20000, maxBuffer: 1024 * 1024});
                assert.equal(stdout.match(/data-result="([^"]*)"/)?.[1], 'passed', 'DPR ' + dpr);
            } finally {
                await rm(profile, {recursive: true, force: true, maxRetries: 5});
            }
        }
    } finally {
        server.closeAllConnections();
        await new Promise(resolve => server.close(resolve));
    }
});

// Capture screenshots for README
const { app, BrowserWindow } = require('electron');
const path = require('path');
const fs = require('fs');

const SCREENSHOT_DIR = path.join(__dirname, '..', 'docs', 'assets');

async function capture(win, name, rect) {
  await new Promise(r => setTimeout(r, 300));
  const image = rect
    ? await win.webContents.capturePage(rect)
    : await win.webContents.capturePage();
  const png = image.toPNG();
  const fp = path.join(SCREENSHOT_DIR, `${name}.png`);
  fs.writeFileSync(fp, png);
  console.log(`  ${name}.png (${(png.length / 1024).toFixed(0)} KB)`);
}

app.whenReady().then(async () => {
  fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });

  const win = new BrowserWindow({
    width: 1400, height: 900,
    titleBarStyle: 'hidden',
    titleBarOverlay: { color: '#1a1a1a', symbolColor: '#cccccc', height: 38 },
    backgroundColor: '#1a1a1a',
    webPreferences: {
      preload: path.join(__dirname, '..', 'dist', 'preload', 'index.js'),
      contextIsolation: true, nodeIntegration: false, sandbox: false, webviewTag: true,
    },
  });

  try { require('../dist/main/ipc-handlers').registerIpcHandlers({ createWindow: () => {}, getAllWindows: () => [{ window: win }] }); } catch {}
  try { require('../dist/main/pipe-server'); } catch {}

  // Mark tutorial as seen so we get the clean app view first
  win.loadURL('http://localhost:5199');
  await new Promise(r => setTimeout(r, 4000));
  await win.webContents.executeJavaScript("localStorage.setItem('pandamux-tutorial-seen', '1')");
  win.reload();

  // Wait for terminals to fully render with PS prompts
  console.log('Waiting for terminals to render...');
  await new Promise(r => setTimeout(r, 10000));

  console.log('Capturing screenshots:');

  // 1. Full app (no tutorial)
  await capture(win, 'pandamux-full');

  // 2. Sidebar crop
  await capture(win, 'pandamux-sidebar', { x: 0, y: 0, width: 260, height: 900 });

  // 3. Terminal area
  await capture(win, 'pandamux-terminals', { x: 260, y: 0, width: 720, height: 900 });

  // 4. Browser panel
  await capture(win, 'pandamux-browser', { x: 960, y: 0, width: 440, height: 900 });

  // 5. Now show tutorial
  await win.webContents.executeJavaScript("localStorage.removeItem('pandamux-tutorial-seen')");
  win.reload();
  await new Promise(r => setTimeout(r, 5000));
  await capture(win, 'pandamux-tutorial');

  // 6. Close tutorial and take one more clean shot
  await win.webContents.executeJavaScript(`
    localStorage.setItem('pandamux-tutorial-seen', '1');
    document.querySelector('.tutorial-overlay')?.remove();
  `);
  await new Promise(r => setTimeout(r, 1000));

  console.log('\nDone! Screenshots in docs/assets/');
  app.quit();
});

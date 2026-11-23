import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { connect } from "node:net";
import { basename, join } from "node:path";
import { setTimeout } from "node:timers/promises";
import { promisify } from "node:util";

const execute = promisify(execFile);
const installer = process.argv[2];
const powershell = process.env.WINDIR + "\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";

async function ps(command) {
  return (await execute(powershell, ["-NoProfile", "-NonInteractive", "-Command", command], { timeout: 30000 })).stdout.trim();
}

async function until(description, condition, timeout = 90000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await condition();
    if (value) return value;
    await setTimeout(250);
  }
  throw new Error("Timed out: " + description);
}

async function listen(server) {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve(server.address().port));
  });
}

let blockDownload = true;
let githubTunnels = 0;
const allowed = new Set(["api.github.com:443", "github.com:443", "release-assets.githubusercontent.com:443", "objects.githubusercontent.com:443"]);
const proxy = createServer();
proxy.on("connect", (request, browser, head) => {
  browser.on("error", () => browser.destroy());
  if (!allowed.has(request.url) || (blockDownload && request.url !== "api.github.com:443")) {
    browser.end("HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
    return;
  }
  githubTunnels++;
  const upstream = connect(443, request.url.split(":")[0]);
  upstream.on("error", () => browser.destroy());
  browser.on("close", () => upstream.destroy());
  upstream.on("connect", () => {
    browser.write("HTTP/1.1 200 Connection Established\r\n\r\n");
    if (head.length) upstream.write(head);
    browser.pipe(upstream).pipe(browser);
  });
});

function openCdp(url) {
  const socket = new WebSocket(url);
  return new Promise((resolve, reject) => {
    socket.addEventListener("error", reject, { once: true });
    socket.addEventListener("open", () => {
      let next = 0;
      const pending = new Map();
      function call(method, params = {}) {
        const id = ++next;
        return new Promise((success, failure) => {
          const timer = globalThis.setTimeout(() => {
            pending.delete(id);
            failure(new Error("WebView2 command timed out: " + method));
          }, 10000);
          pending.set(id, { success, failure, timer });
          socket.send(JSON.stringify({ id, method, params }));
        });
      }
      socket.addEventListener("message", ({ data }) => {
        const msg = JSON.parse(data);
        if (msg.method === "Page.javascriptDialogOpening") {
          void call("Page.handleJavaScriptDialog", { accept: true }).catch(reject);
        }
        const reply = pending.get(msg.id);
        if (!reply) return;
        globalThis.clearTimeout(reply.timer);
        pending.delete(msg.id);
        if (msg.error) reply.failure(new Error(msg.error.message));
        else reply.success(msg.result);
      });
      resolve({ call, close: () => socket.close() });
    }, { once: true });
  });
}

let app;
let cdp;
async function start() {
  const reserve = createServer();
  const port = await listen(reserve);
  await new Promise((resolve) => reserve.close(resolve));
  app = spawn(join(process.env.LOCALAPPDATA, "FontFerry", "fontferry.exe"), [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=" + port },
    stdio: "ignore",
  });
  let launchError;
  app.once("error", (error) => { launchError = error; });
  const url = await until("FontFerry WebView2", async () => {
    if (launchError) throw launchError;
    if (app.exitCode !== null) throw new Error("FontFerry exited: " + app.exitCode);
    try {
      const pages = await (await fetch("http://127.0.0.1:" + port + "/json/list")).json();
      return pages.find((page) => page.type === "page" && /tauri\.localhost/.test(page.url))?.webSocketDebuggerUrl;
    } catch {
      return null;
    }
  });
  cdp = await openCdp(url);
  await cdp.call("Page.enable");
}

async function stop() {
  cdp?.close();
  if (app && app.exitCode === null) {
    await ps("if ($app = Get-Process -Id " + app.pid + " -ErrorAction SilentlyContinue) { [void]$app.CloseMainWindow(); if (!$app.WaitForExit(10000)) { throw 'FontFerry did not exit' } }");
  }
}

async function evaluate(expression) {
  const response = await cdp.call("Runtime.evaluate", { expression, returnByValue: true });
  if (response.exceptionDetails) throw new Error(response.exceptionDetails.text);
  return response.result.value;
}

async function click(text, scope = "body") {
  await until("button " + text, () => evaluate(
    "(() => { const button = [...document.querySelectorAll(" + JSON.stringify(scope + " button") +
    ")].find((item) => !item.disabled && item.textContent.trim() === " + JSON.stringify(text) +
    "); if (!button) return false; button.click(); return true; })()",
  ));
}

async function shown(text, scope = "body") {
  try {
    await until(text, () => evaluate(
      "document.querySelector(" + JSON.stringify(scope) + ")?.innerText.includes(" + JSON.stringify(text) + ")",
    ));
  } catch (error) {
    console.error(await evaluate("document.body.innerText"));
    throw error;
  }
}

const drawer = '[role="dialog"]';

async function openFont(name) {
  const catalogOpen = await evaluate("document.querySelector('h1')?.textContent.trim() === '字体'");
  if (!catalogOpen) await click("浏览字体");
  await until(name + " 条目", () => evaluate(
    "(() => { const item = [...document.querySelectorAll('button')].find((button) => button.textContent.includes(" +
    JSON.stringify(name) + ")); if (!item) return false; item.click(); return true; })()",
  ));
  await shown(name, drawer);
}

async function chooseSmiley() {
  await click("安装字体", drawer);
  await shown("smiley-sans-", drawer);
  await click("确认安装", drawer);
}

async function verifyRegistered() {
  const registered = await ps("Add-Type -AssemblyName System.Drawing; $fonts = New-Object Drawing.Text.InstalledFontCollection; $fonts.Families | ForEach-Object { $_.GetName(1033) } | Where-Object { $_ -match 'Smiley' }");
  assert.match(registered, /Smiley/i);
  console.log("GDI font selection: " + registered);
}

try {
  assert.ok(installer);
  assert.match(await ps("[Security.Principal.WindowsIdentity]::GetCurrent().Name"), new RegExp("\\\\" + process.argv[3] + "$", "i"));
  const administrator = await ps("$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()); $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)");
  assert.equal(administrator, "False", "Native product smoke must run as a standard user");
  // Start-Process can inherit the launching user's environment despite a different token.
  const folders = JSON.parse(await ps("$create = [Environment+SpecialFolderOption]::Create; @{ profile = [Environment]::GetFolderPath('UserProfile', $create); local = [Environment]::GetFolderPath('LocalApplicationData', $create); roaming = [Environment]::GetFolderPath('ApplicationData', $create) } | ConvertTo-Json -Compress"));
  assert.equal(basename(folders.profile).toLowerCase(), process.argv[3].toLowerCase());
  // The freshly created CI account has never initialized Explorer's shell folders.
  const shellFolders = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders";
  for (const [key, value, defaultName] of [["local", "Local AppData", "Local"], ["roaming", "AppData", "Roaming"]]) {
    if (folders[key]) continue;
    folders[key] = join(folders.profile, "AppData", defaultName);
    await mkdir(folders[key], { recursive: true });
    const folder = folders[key].replaceAll("'", "''");
    await ps("$ErrorActionPreference='Stop'; $key='" + shellFolders + "'; if (!(Test-Path -LiteralPath $key)) { New-Item -Path $key -Force | Out-Null }; " +
      "New-ItemProperty -LiteralPath $key -Name '" + value + "' -Value '" + folder + "' -PropertyType ExpandString -Force | Out-Null");
  }
  process.env.USERPROFILE = folders.profile;
  process.env.LOCALAPPDATA = folders.local;
  process.env.APPDATA = folders.roaming;
  process.env.USERNAME = process.argv[3];
  process.env.TEMP = join(folders.local, "Temp");
  process.env.TMP = process.env.TEMP;
  await mkdir(process.env.TEMP, { recursive: true });
  console.log("Verified standard-user Windows profile");
  await execute(installer, ["/S", "/D=" + join(folders.local, "FontFerry")], { timeout: 120000 });
  const port = await listen(proxy);
  await start();
  await openFont("得意黑");
  await chooseSmiley();
  await shown("安装完成", drawer);
  await verifyRegistered();
  console.log("PASS A01: direct installation in a fresh, standard Windows user profile");
  await click("卸载字渡管理的文件", drawer);
  await shown("已卸载字渡管理的字体文件");

  await until("设置", () => evaluate(
    "(() => { const item = [...document.querySelectorAll('nav button')].find((button) => button.textContent.includes('设置')); if (!item) return false; item.click(); return true; })()",
  ));
  await shown("本机 HTTP 代理端口");
  await evaluate("(() => { const input = document.querySelector('input[type=number]'); input.value = " + port + "; input.dispatchEvent(new Event('input', { bubbles: true })); })()");
  await click("保存代理设置");
  await shown("代理设置已保存");
  await stop();

  await start();
  await openFont("得意黑");
  await chooseSmiley();
  await shown("下载字体失败", drawer);
  await shown("打开网络设置", drawer);
  assert.ok(githubTunnels > 0, "No probe used the supplied proxy");
  blockDownload = false;
  await click("确认安装", drawer);
  await shown("安装完成", drawer);
  await verifyRegistered();
  console.log("PASS A02: metadata-only proxy failure, then in-place retry and native registration");

  const fontDirectory = join(process.env.LOCALAPPDATA, "Microsoft", "Windows", "Fonts");
  const managedNames = (await readdir(fontDirectory)).filter((name) => /^smiley-sans-.+\.(ttf|otf)$/i.test(name));
  assert.ok(managedNames.length, "No owned files in the Windows user font directory");
  const bytes = await readFile(join(fontDirectory, managedNames[0]));
  await click("卸载字渡管理的文件", drawer);
  await shown("已卸载字渡管理的字体文件");
  const control = join(fontDirectory, "unmanaged-smiley." + managedNames[0].split(".").at(-1));
  await writeFile(control, bytes, { flag: "wx" });
  const registry = "HKCU:\\Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
  const escaped = control.replaceAll("'", "''");
  await ps("New-ItemProperty -LiteralPath '" + registry + "' -Name 'External Smiley Test' -Value '" + escaped + "' -ErrorAction Stop | Out-Null");
  const filesBefore = (await readdir(fontDirectory)).sort();
  await openFont("得意黑");
  await chooseSmiley();
  await shown("系统中已有同名字体", drawer);
  assert.deepEqual(await readFile(control), bytes);
  assert.deepEqual((await readdir(fontDirectory)).sort(), filesBefore);
  assert.equal(await ps("Get-ItemPropertyValue -LiteralPath '" + registry + "' -Name 'External Smiley Test'"), control);
  assert.equal(await evaluate("document.querySelector('[role=dialog]').textContent.includes('卸载字渡管理的文件')"), false);
  console.log("PASS A04: pre-write conflict preserved external bytes, registry entry, and ownership state");

  await evaluate("document.querySelector('button[aria-label=关闭]').click()");
  await openFont("Maple Mono");
  for (const name of ["MapleMono-NF-CN", "MapleMono-TTF"]) {
    assert.equal(await evaluate(
      "(() => { const label = [...document.querySelectorAll('[role=dialog] label')].find((item) => item.querySelector('strong')?.textContent === " +
      JSON.stringify(name) + "); if (!label) return false; label.querySelector('input').click(); return true; })()",
    ), true, "Variant not found: " + name);
  }
  await click("安装字体", drawer);
  await shown("MapleMono-TTF.zip", drawer);
  assert.equal(await evaluate("document.querySelector('[role=dialog]').innerText.includes('MapleMono-NF-CN.zip')"), false);
  await click("确认安装", drawer);
  await shown("安装完成", drawer);
  console.log("PASS A03: real installer honored the non-default Maple Mono TTF variant");
} finally {
  try { await stop(); } finally { proxy.close(); }
}

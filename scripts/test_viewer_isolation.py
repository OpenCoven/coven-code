#!/usr/bin/env python3
"""Browser regression using an installed playwright-cli (or its wrapper).

No repository dependency is installed. Browser offline mode and fixture-only CSP
block external resources; mocked routes supply the viewer and gist. The actual
legacy templates exercise their no-CDN rendering fallback with a synthetic session.
"""
import argparse
import base64
import json
from pathlib import Path
import subprocess
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cli', default='playwright-cli')
    parser.add_argument('--viewer', type=Path, default=ROOT / 'session/index.html')
    args = parser.parse_args()
    export_dir = ROOT / 'src-rust/crates/core/src/share_export'
    payload = {'meta': {'title': 'Fixture caf\u00e9 \u6771\u4eac \U00010000'}, 'messages': [
        {'role': 'user', 'content': 'Fixture prose caf\u00e9 \u6771\u4eac \U00010000 </script><b>literal</b>'},
        {'role': 'assistant', 'content': [{'type': 'tool_use', 'name': 'FixtureTool', 'input': {}}]},
    ]}
    export = (export_dir / 'template.html').read_text()
    replacements = {
        '{{TITLE}}': 'Fixture caf\u00e9 \u6771\u4eac \U00010000', '{{CSS}}': (export_dir / 'template.css').read_text(),
        '{{JS}}': (export_dir / 'template.js').read_text(),
        '{{SESSION_DATA}}': base64.b64encode(json.dumps(payload).encode()).decode(),
    }
    for key, value in replacements.items():
        export = export.replace(key, value)
    export = export.replace('<body>', '<body data-bootstrap-token="</script>">')
    code = 'async (page) => {\nconst viewerHtml = ' + json.dumps(args.viewer.read_text()) + ';\n'
    code += 'const exportHtml = ' + json.dumps(export) + ';\n' + r'''
const origin = 'http://127.0.0.1:48173';
await page.context().setOffline(true);
const csp = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src https://api.github.com; frame-src 'self' about:";
const key = 'coven-code.share.showTools';
const protocol = 'coven-code.share.tools-visibility';
const pageErrors = [];
const routed = [];
const finished = [];
page.context().on('requestfinished', request => finished.push(request.url()));
page.on('pageerror', error => pageErrors.push(String(error)));
await page.addInitScript(() => {
  if (window === window.top) {
    DOMParser.prototype.parseFromString = () => { throw new Error('parent parsed untrusted HTML'); };
  }
});
function check(value, label) { if (!value) throw new Error(label); }
await page.context().route('**/*', route => {
  const url = new URL(route.request().url());
  routed.push(url.href);
  if (url.origin === origin && ['/session/', '/blocked', '/standalone'].includes(url.pathname)) {
    return route.fulfill({contentType:'text/html', headers:{'content-security-policy':csp}, body: url.pathname === '/standalone' ? exportHtml : viewerHtml});
  }
  if (url.href === 'https://api.github.com/gists/deadbeef') return route.fulfill({
    contentType:'application/json', headers:{'access-control-allow-origin':'*'},
    body:JSON.stringify({files:{'session.html':{filename:'session.html',content:exportHtml}}})});
  return route.abort();
});
async function loaded(path = '/session/#deadbeef') {
  await page.goto(origin + path);
  const toggle = page.frameLocator('#viewer').locator('.hero-controls input[type=checkbox]');
  await toggle.waitFor({timeout:5000});
  const frame = page.frames().find(frame => frame.parentFrame() === page.mainFrame());
  await frame.waitForLoadState('domcontentloaded');
  check(await frame.evaluate(() => typeof marked === 'undefined' && typeof hljs === 'undefined'), 'external scripts loaded: '+JSON.stringify({routed,finished}));
  check(await frame.locator('.hero-title').textContent() === 'Fixture caf\u00e9 \u6771\u4eac \u{10000}', 'title rendering');
  check((await frame.locator('#messages').textContent()).includes('Fixture prose caf\u00e9 \u6771\u4eac \u{10000} </script><b>literal</b>'), 'prose rendering: ' + JSON.stringify({text:await frame.locator('#messages').textContent(), globals:await frame.evaluate(() => [typeof marked,typeof hljs]), routed,finished}));
  check(await frame.locator('.tool-call').count() === 1, 'tool rendering');
  check(await frame.locator('body').getAttribute('data-bootstrap-token') === '</script>', 'bootstrap data escaped');
  check(await frame.evaluate(() => document.compatMode) === 'CSS1Compat', 'standards mode');
  return {frame, toggle};
}
let {frame, toggle} = await loaded();
await page.evaluate(key => localStorage.removeItem(key), key);
const denied = await frame.evaluate(() => {
  let dom = false, storage = false;
  try { parent.document.body.dataset.probe = 'bad'; dom = true; } catch (_) {}
  try { parent.localStorage.setItem('unrelated', 'bad'); storage = true; } catch (_) {}
  return {dom, storage};
});
check(!denied.dom && !denied.storage, 'iframe accessed parent DOM/storage');
await toggle.locator('..').click({timeout:5000});
await page.waitForFunction(key => localStorage.getItem(key) === '0', key, {timeout:5000});
check(await frame.locator('#messages').evaluate(el => el.classList.contains('hide-tools')), 'hide tools');
({frame, toggle} = await loaded());
check(!await toggle.isChecked(), 'legacy preference did not survive reload');
await toggle.locator('..').click({timeout:5000});
await page.waitForFunction(key => localStorage.getItem(key) === '1', key, {timeout:5000});
check(!await frame.locator('#messages').evaluate(el => el.classList.contains('hide-tools')), 'show tools');
// FIFO message dispatch: a valid sentinel follows invalid messages. Its storage
// write acknowledges the entire batch without timing sleeps or polling silence.
await page.evaluate(({key, protocol}) => {
  window.__invalidWrites = [];
  const native = Storage.prototype.setItem;
  Storage.prototype.setItem = function(k, v) {
    window.__invalidWrites.push([k, v]);
    return native.call(this, k, v);
  };
  window.postMessage({type:protocol,showTools:false}, location.origin);
  const child = document.getElementById('viewer').contentWindow;
  window.dispatchEvent(new MessageEvent('message', {
    source:child,origin:location.origin,data:{type:protocol,showTools:false}}));
}, {key, protocol});
await frame.evaluate(protocol => {
  for (const message of [null, [], {type:'wrong',showTools:false},
    {type:protocol,showTools:'false'}, {type:protocol,showTools:false,key:'unrelated'}]) {
    parent.postMessage(message, '*');
  }
  parent.postMessage({type:protocol,showTools:true}, '*');
}, protocol);
await page.waitForFunction(() => window.__invalidWrites.some(entry => entry[1] === '1'), null, {timeout:5000});
const writes = await page.evaluate(() => window.__invalidWrites);
check(JSON.stringify(writes) === JSON.stringify([[key,'1']]), 'invalid sender/message wrote storage: '+JSON.stringify(writes));
// An opaque sibling is not the active viewer, even though its origin is null.
await page.evaluate(protocol => {
  window.__siblingDone = false;
  window.addEventListener('message', event => {
    if (event.data === 'sibling-done') window.__siblingDone = true;
  });
  const sibling = document.createElement('iframe');
  sibling.sandbox = 'allow-scripts';
  sibling.srcdoc = '<script>parent.postMessage({type:' + JSON.stringify(protocol) +
    ',showTools:false},"*");parent.postMessage("sibling-done","*");<\/script>';
  document.body.appendChild(sibling);
}, protocol);
await page.waitForFunction(() => window.__siblingDone, null, {timeout:5000});
check(JSON.stringify(await page.evaluate(() => window.__invalidWrites)) === JSON.stringify([[key,'1']]), 'sibling wrote storage');
// Storage failures remain nonfatal; the toggle still controls this document.
await page.addInitScript(() => {
  if (location.pathname === '/blocked') {
    Object.defineProperty(window, 'localStorage', {get() { throw new Error('fixture blocked'); }});
  }
});
({frame, toggle} = await loaded('/blocked#deadbeef'));
check(await toggle.isChecked(), 'storage failure default');
await page.evaluate(() => {
  window.__changeDelivered = false;
  window.addEventListener('message', event => {
    if (event.data && event.data.type === 'coven-code.share.tools-visibility') window.__changeDelivered = true;
  });
});
await toggle.locator('..').click({timeout:5000});
await page.waitForFunction(() => window.__changeDelivered, null, {timeout:5000});
check(await frame.locator('#messages').evaluate(el => el.classList.contains('hide-tools')), 'storage failure toggle');
check(pageErrors.length === 0, 'uncaught page errors: '+JSON.stringify(pageErrors));
// Standalone exports have not acquired a dependency on a parent bridge.
await page.goto(origin + '/standalone');
const standalone = page.locator('.hero-controls input[type=checkbox]');
await standalone.locator('..').click({timeout:5000});
await page.reload();
check(!await standalone.isChecked(), 'standalone preference');
return 'viewer isolation, legacy persistence, message validation, storage failure, standalone: PASS';
}'''
    session = 'viewer-' + uuid.uuid4().hex[:12]
    with tempfile.TemporaryDirectory(prefix='coven-viewer-test-') as work:
        command = [args.cli, '--session', session]
        try:
            subprocess.run(command + ['open', 'about:blank'], cwd=work, check=True, timeout=30,
                           stdout=subprocess.DEVNULL)
            result = subprocess.run(command + ['run-code', code], cwd=work, text=True,
                                    capture_output=True, timeout=60)
            expected = '### Result\n' + json.dumps('viewer isolation, legacy persistence, message validation, storage failure, standalone: PASS')
            if result.returncode or expected not in result.stdout or '### Error' in result.stdout:
                print(result.stdout[-4000:])
                print(result.stderr[-4000:], end='')
                raise SystemExit(1)
            print(expected)
        finally:
            subprocess.run(command + ['close'], cwd=work, timeout=30, check=False,
                           stdout=subprocess.DEVNULL)


if __name__ == '__main__':
    main()

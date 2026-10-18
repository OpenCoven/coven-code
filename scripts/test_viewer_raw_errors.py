#!/usr/bin/env python3
"""Exercise the real viewer with offline Chromium and exact mocked HTTP routes.

Requires an installed playwright-cli or its wrapper; adds no repository dependency.
Fixture-only CSP blocks external resources even if opaque-frame routing misses them.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[1]
PASS = 'raw HTTP errors, complete raw content, inline content, network failure: PASS'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cli', default='playwright-cli')
    args = parser.parse_args()
    code = 'async (page) => {\nconst html = ' + json.dumps((ROOT / 'session/index.html').read_text()) + ';\n'
    code += r'''
const origin = 'http://127.0.0.1:48174';
const api = 'https://api.github.com/gists/deadbeef';
const raw = 'https://gist.githubusercontent.com/fixture/deadbeef/raw/session.html';
const partial = '<!DOCTYPE html><p id="payload">PARTIAL CONTENT</p>';
const complete = '<!DOCTYPE html><p id="payload">COMPLETE CONTENT</p>';
const csp = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src https://api.github.com https://gist.githubusercontent.com; frame-src 'self' about:";
let scenario;
let rawRequests;
let apiRequests;
const errors = [];
const outcomes = [];
await page.context().setOffline(true);
await page.context().route('**/*', route => {
  const url = route.request().url();
  if (url === origin + '/session/') return route.fulfill({contentType:'text/html', body:html,
    headers:{'content-security-policy':csp}});
  if (url === api) {
    apiRequests++;
    return route.fulfill({contentType:'application/json', headers:{'access-control-allow-origin':'*'},
      body:JSON.stringify({files:{'session.html':{filename:'session.html', content:partial,
        truncated:scenario !== 'inline', raw_url:raw}}})});
  }
  if (url === raw) {
    rawRequests++;
    if (scenario === 'abort') return route.abort();
    return route.fulfill({status: scenario === 'complete' ? 200 : Number(scenario),
      contentType:'text/html', headers:{'access-control-allow-origin':'*'},
      body:scenario === 'complete' ? complete : '<p>PRIVATE ERROR BODY</p>'});
  }
  return route.abort();
});
function check(value, message) { if (!value) throw new Error(message); }
for (scenario of ['404', '500', 'complete', 'inline', 'abort']) {
  rawRequests = 0;
  apiRequests = 0;
  try {
    await page.goto('about:blank');
    await page.goto(origin + '/session/#deadbeef');
    await page.waitForFunction(() => document.body.classList.contains('viewer-ready') ||
      document.getElementById('status-title').textContent !== 'Loading session', null, {timeout:5000});
    const ready = await page.locator('body').evaluate(el => el.classList.contains('viewer-ready'));
    const status = await page.locator('#status-title').textContent();
    const detail = await page.locator('#status-detail').textContent();
    let rendered = null;
    if (ready) {
      const payload = page.frameLocator('#viewer').locator('#payload');
      await payload.waitFor({state:'visible',timeout:5000});
      rendered = await payload.textContent();
    }
    check(apiRequests === 1, 'expected one gist request');
    check(rawRequests === (scenario === 'inline' ? 0 : 1), 'unexpected raw request count');
    if (scenario === '404' || scenario === '500') {
      check(!ready && status === 'Unable to load complete session',
        JSON.stringify({ready,status,detail,rendered}));
      check(detail === 'GitHub returned status ' + scenario + ' for the complete session file.', 'HTTP status detail');
      check(await page.locator('#viewer').getAttribute('srcdoc') === null, 'partial content assigned');
      check(!detail.includes('PRIVATE ERROR BODY'), 'response body disclosed');
    } else if (scenario === 'abort') {
      check(!ready && status === 'Network error' && detail.length > 0, 'network error behavior');
      check(await page.locator('#viewer').getAttribute('srcdoc') === null, 'content assigned after network failure');
    } else {
      check(ready && rendered === (scenario === 'complete' ? 'COMPLETE CONTENT' : 'PARTIAL CONTENT'), 'wrong rendered content');
    }
    outcomes.push(scenario + ': PASS');
  } catch (error) {
    errors.push(scenario + ': ' + String(error));
  }
}
if (errors.length) throw new Error(JSON.stringify({outcomes,errors}));
return 'raw HTTP errors, complete raw content, inline content, network failure: PASS';
}'''
    command = [args.cli, '--session', 'raw-' + uuid.uuid4().hex[:12]]
    with tempfile.TemporaryDirectory(prefix='coven-viewer-raw-') as work:
        try:
            subprocess.run(command + ['open', 'about:blank'], cwd=work, check=True, timeout=30,
                           stdout=subprocess.DEVNULL)
            result = subprocess.run(command + ['run-code', code], cwd=work, text=True,
                                    capture_output=True, timeout=60)
            if result.returncode or ('### Result\n' + json.dumps(PASS)) not in result.stdout or '### Error' in result.stdout:
                print(result.stdout[-4000:])
                print(result.stderr[-4000:], end='')
                raise SystemExit(1)
            print(PASS)
        finally:
            subprocess.run(command + ['close'], cwd=work, timeout=30, check=False,
                           stdout=subprocess.DEVNULL)


if __name__ == '__main__':
    main()

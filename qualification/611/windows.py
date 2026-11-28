import hashlib, http.server, json, os, queue, re, shutil, subprocess, sys, tempfile, threading, time, zipfile
from pathlib import Path
from winpty import PtyProcess
root, binaries, assets = map(lambda p: Path(p).resolve(), sys.argv[1:4])
archive = assets / 'coven-code-windows-x86_64.zip'
def digest(p):
 with p.open('rb') as f: return hashlib.file_digest(f, 'sha256').hexdigest()
with zipfile.ZipFile(archive) as z:
 with z.open(next(f for f in z.namelist() if Path(f).name == 'coven-code.exe')) as f: expected = hashlib.file_digest(f,'sha256').hexdigest()
def run(ui, mode):
 p=root/(ui+'-'+mode); p.mkdir(parents=True); exe=p/'coven-code.exe'
 shutil.copy2(binaries/'fixture'/'coven-code.exe',exe); before=digest(exe)
 config=p/'.coven-code';config.mkdir();(config/'settings.json').write_text('{"hasCompletedOnboarding":true}')
 requests=[]
 class Release(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   requests.append(self.path)
   if self.path.endswith('/latest'): data=json.dumps({'tag_name':'v0.9.2' if mode=='current' else 'v0.9.3'}).encode()
   elif self.path.endswith('/SHA256SUMS'): data=((assets/'SHA256SUMS').read_bytes() if mode!='reject' else ('0'*64+'  '+archive.name+'\n').encode())
   elif self.path.endswith('/'+archive.name): data=archive.read_bytes()
   else: self.send_error(404); return
   self.send_response(200); self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 server=http.server.ThreadingHTTPServer(('127.0.0.1',41611),Release)
 threading.Thread(target=server.serve_forever,daemon=True).start()
 env={k:v for k,v in os.environ.items() if k.upper() in ('PATH','SYSTEMROOT','WINDIR','TEMP','TMP','COMSPEC','PATHEXT','PROCESSOR_ARCHITECTURE')}
 env.update(HOME=str(p),USERPROFILE=str(p),APPDATA=str(p/'roaming'),LOCALAPPDATA=str(p/'local'),COVEN_CODE_HOME=str(config),COVEN_CODE_UI=ui,COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1',TERM='xterm-256color')
 proc=PtyProcess.spawn([str(exe)],cwd=str(p),env=env,dimensions=(40,120))
 chunks=[]
 def read():
  try:
   while True:
    part=proc.read(65536)
    if not part:break
    chunks.append(part)
  except (EOFError,OSError):pass
 threading.Thread(target=read,daemon=True).start()
 def wait(pred,timeout=35):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   if pred():return
   time.sleep(.1)
  raise AssertionError((ui,mode,requests,''.join(chunks)[-3000:]))
 try:
  wait(lambda:any(w in ''.join(chunks) for w in ['shortcuts','preview','Coven v']))
  time.sleep(.6);proc.write('/update');time.sleep(.4);proc.write('\r')
  if mode=='current':
   wait(lambda:'up to date' in ''.join(chunks));proc.write('\x03');time.sleep(.3);proc.write('\x03')
  wait(lambda:not proc.isalive());time.sleep(.4)
  raw=''.join(chunks);(p/'terminal.txt').write_text(raw,encoding='utf-8')
  version=subprocess.check_output([str(exe),'--version'],text=True).strip();after=digest(exe)
  if mode=='upgrade':assert version=='coven-code 0.9.3' and after==expected and after!=before
  else:assert version=='coven-code 0.9.2' and before==after
  if mode=='reject':assert 'checksum mismatch' in raw.lower()
  print(ui,mode,version,before,after,requests,flush=True)
 finally:
  if proc.isalive():proc.terminate(force=True)
  server.shutdown();server.server_close()
for ui in ['classic','next']:
 for mode in ['current','reject','upgrade']:run(ui,mode)

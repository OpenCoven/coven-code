import base64, hashlib, http.server, json, os, pathlib, shlex, shutil, socketserver, subprocess, sys, tarfile, tempfile, threading, time
from http.server import BaseHTTPRequestHandler
from pathlib import Path
root, binary, assets=map(lambda x:Path(x).resolve(),sys.argv[1:4])
archive=assets/sys.argv[4]
def digest(path):
 with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
with tarfile.open(archive) as t:
 member=next(m for m in t if Path(m.name).name=='coven-code' and m.isfile())
 expected=hashlib.file_digest(t.extractfile(member),'sha256').hexdigest()
class Ledger(socketserver.ThreadingUnixStreamServer): daemon_threads=True
def test(ui,mode):
 p=root/(ui+'-'+mode);p.mkdir()
 exe=p/'coven-code';shutil.copy2(binary,exe);before=digest(exe)
 (p/'.coven-code').mkdir();(p/'.coven-code/settings.json').write_text('{"hasCompletedOnboarding":true,"daemonLedger":true}')
 events=[]
 class Handler(BaseHTTPRequestHandler):
  def log_message(self,*args):pass
  def answer(self,body):
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def do_GET(self):
   events.append(self.path)
   if self.path.startswith('/api/v1/'): self.answer(b'{"instance_id":"fixture-instance","pid":123,"status":"ok"}')
   elif self.path.endswith('/latest'): self.answer(json.dumps({'tag_name':'v0.9.2' if mode=='current' else 'v0.9.3'}).encode())
   elif self.path.endswith('/SHA256SUMS'): self.answer((assets/'SHA256SUMS').read_bytes() if mode!='reject' else ('0'*64+'  '+archive.name+'\n').encode())
   elif self.path.endswith('/'+archive.name): self.answer(archive.read_bytes())
   else:self.send_error(404)
  def do_POST(self):
   self.rfile.read(int(self.headers.get('content-length','0')));events.append(self.path);self.answer(b'{"id":"fixture-session","session_id":"fixture-session"}')
 sock=p/'coven.sock'
 ledger=Ledger(str(sock),Handler);ledger.server_name='localhost';ledger.server_port=0
 web=http.server.ThreadingHTTPServer(('127.0.0.1',41611),Handler)
 for srv in [ledger,web]:threading.Thread(target=srv.serve_forever,daemon=True).start()
 tag='u611-'+str(os.getpid())+ui+mode
 def tm(*a,check=True):return subprocess.run(['tmux','-L',tag,*a],capture_output=True,text=True,check=check)
 def pane():return tm('capture-pane','-p','-S','-200','-t','s',check=False).stdout
 def wait(fn,sec):
  end=time.monotonic()+sec
  while time.monotonic()<end:
   if fn():return
   time.sleep(.08)
  raise AssertionError((events,pane()))
 try:
  env={'HOME':str(p),'PATH':os.environ['PATH'],'LANG':'en_US.UTF-8','TERM':'xterm-256color','COVEN_CODE_UI':ui,'COVEN_HOME':str(p),'COVEN_DAEMON_SOCKET':str(sock),'COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1'}
  cmd='env -i '+shlex.join([k+'='+v for k,v in env.items()]+[str(exe)])
  tm('new-session','-d','-s','s','-x','120','-y','40','-c',str(p),'sleep 0.5; '+cmd+'; echo $? > exit-code; sleep 80')
  tm('pipe-pane','-t','s','cat > '+shlex.quote(str(p/'tty')))
  wait(lambda:'shortcuts' in pane() or 'Coven v' in pane() or 'Coven · preview' in pane(),22)
  time.sleep(.4)
  tm('send-keys','-l','/update');time.sleep(.25);tm('send-keys','Enter')
  if mode=='current':
   wait(lambda:'up to date' in pane(),12)
   tm('send-keys','C-c');time.sleep(.15);tm('send-keys','C-c')
  wait(lambda:(p/'exit-code').exists(),30)
  output=(p/'tty').read_bytes();rc=int((p/'exit-code').read_text());version=subprocess.check_output([exe,'--version'],text=True).strip();after=digest(exe)
  assert b'\x1b[?1049l' in output and b'\x1b[?25h' in output,(p,output[-700:])
  if mode!='current':
   assert output.index(b'\x1b[?1049l')<output.index(b'Downloading '),p
   assert any('complet' in e for e in events[:events.index(next(e for e in events if e.endswith(archive.name)))]),events
  if mode=='upgrade':assert rc==0 and version=='coven-code 0.9.3' and after==expected and before!=after,(rc,version,events)
  else:assert after==before and version=='coven-code 0.9.2',(after,version)
  if mode=='reject':assert rc!=0 and b'checksum mismatch' in output.lower(),(rc,output[-800:])
  print(ui,mode,rc,version,before,after,events,flush=True)
 finally:
  tm('kill-server',check=False)
  for srv in [web,ledger]:srv.shutdown();srv.server_close()
for ui in ['classic','next']:
 for mode in ['current','reject','upgrade']:test(ui,mode)

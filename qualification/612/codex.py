import os,sys,time,subprocess,json,shlex,tempfile,pathlib,base64
P=pathlib.Path
binary=P(sys.argv[1]).resolve()
import http.server,urllib.parse,hashlib,threading
state={};seen=[];refreshed=[];requested=[];challenges={}
def enc(v):return base64.urlsafe_b64encode(v).decode().rstrip('=')
def jwt(who):return 'e30.'+enc(json.dumps({'https://api.openai.com/auth':{'account_id':who}}).encode())+'.fixture'
class Server(http.server.BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def reply(self,body,typ='application/json'):
  self.send_response(200);self.send_header('Content-Type',typ);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def do_GET(self):
  q=dict(urllib.parse.parse_qsl(urllib.parse.urlparse(self.path).query));who=state['who'];challenges[who]=q['code_challenge']
  self.send_response(302);self.send_header('Location',q['redirect_uri']+'?'+urllib.parse.urlencode({'state':q['state'],**({'error':'access_denied'} if who=='cancel' else {'code':who})}));self.end_headers()
 def do_POST(self):
  b=self.rfile.read(int(self.headers['Content-Length']))
  if self.path.endswith('token'):
   if b.startswith(b'{'):q=json.loads(b)
   else:q=dict(urllib.parse.parse_qsl(b.decode()))
   if 'code' in q:
    who=q['code'];assert enc(hashlib.sha256(q['code_verifier'].encode()).digest())==challenges[who];requested.append(who)
   else:who=q['refresh_token'].removeprefix('fixture-');refreshed.append(who)
   self.reply(json.dumps({'access_token':jwt(who),'refresh_token':'fixture-'+who,'expires_in':7200}).encode())
  else:
   who=self.headers.get('ChatGPT-Account-Id');assert self.headers.get('Authorization')=='Bearer '+jwt(who);seen.append(who)
   self.reply(('event: response.output_text.delta\ndata: '+json.dumps({'delta':'fixture reply '+who})+'\n\nevent: response.completed\ndata: {"response":{"id":"fixture"}}\n\n').encode(),'text/event-stream')
srv=http.server.ThreadingHTTPServer(('127.0.0.1',41612),Server);threading.Thread(target=srv.serve_forever,daemon=True).start()
for ui in ['classic','next']:
 p=P(tempfile.mkdtemp(prefix='acct612-',dir='/tmp'));home=p/'engine';home.mkdir()
 (home/'settings.json').write_text('{"hasCompletedOnboarding":true}')
 (p/'familiars.toml').write_text('[[familiar]]\nid="dev"\nname="dev"\n[[familiar]]\nid="writer"\nname="writer"\n')
 tag=p.name
 def tm(*args,check=True):return subprocess.run(['tmux','-L',tag,*args],capture_output=True,text=True,check=check)
 def pane():return tm('capture-pane','-p','-t','s',check=False).stdout
 def wait(f):
  for _ in range(120):
   if f():return
   time.sleep(.08)
  raise AssertionError((ui,str(p),'timed out at '+str(sys._getframe(1).f_lineno)))
 def start(agent):
  global tag
  tag=p.name+'-'+str(time.monotonic_ns())
  env={'HOME':str(p),'COVEN_HOME':str(p),'COVEN_CODE_HOME':str(home),'COVEN_CODE_UI':ui,'COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1','PATH':os.environ['PATH'],'LANG':'C.UTF-8','TERM':'xterm-256color'}
  cmd='exec env -i '+shlex.join([k+'='+v for k,v in env.items()]+[str(binary),'--agent',agent,'--model','codex/gpt-5-codex'])
  tm('new-session','-d','-x','140','-y','45','-s','s','-c',str(p),cmd);wait(lambda:any(x in pane() for x in ['shortcuts','preview','Coven v']));time.sleep(.45)
 def send(line):
  tm('send-keys','-l',line);time.sleep(.25);tm('send-keys','Enter');time.sleep(.45)
 def stop():tm('kill-server',check=False)
 def users():return json.loads((home/'accounts.json').read_text())['providers']['codex']['profiles']
 def count():return len((p/'observed').read_text().splitlines()) if (p/'observed').exists() else 0
 try:
  start('dev')
  for i,who in enumerate(['A','B'],1):
   state['who']=who;send('/account login codex --label work');wait(lambda:(home/'accounts.json').exists() and len(users())==i)
   wait(lambda:tm('has-session','-t','s',check=False).returncode!=0);start('dev')
  assert set(users())=={'work','work-2'}
  state['who']='cancel';send('/account login codex --label cancel');wait(lambda:'cancel' in challenges);assert len(users())==2
  send('/account list --viable')
  bind=home/'familiar-accounts'/'ZGV2.json'
  send('/familiar account dev codex/work');wait(bind.exists);assert json.loads(bind.read_text())['id']=='work'
  send('/account switch codex/work-2');wait(lambda:tm('has-session','-t','s',check=False).returncode!=0)
  def turn(agent,expected):
   start(agent);n=len(seen);send('identity check');wait(lambda:len(seen)>n);assert seen[-1]==expected;time.sleep(.3);print(ui,agent,expected,'observed',seen[-1],flush=True);stop()
  turn('dev','A');turn('writer','B')
  tok=home/'accounts/codex/work/codex_tokens.json';second=home/'accounts/codex/work-2/codex_tokens.json';before=second.read_bytes()
  def expire(remove):
   t=json.loads(tok.read_text());t['expires_at']=1
   if remove:t['refresh_token']=None
   tok.write_text(json.dumps(t))
  expire(False);n=len(refreshed);turn('dev','A');assert refreshed[n:]==['A'] and second.read_bytes()==before
  expire(True);start('dev');n=len(seen);send('identity check');wait(lambda:'unavailable' in pane());assert len(seen)==n
  send('/familiar account dev codex/work-2');wait(lambda:json.loads(bind.read_text())['id']=='work-2');stop();turn('dev','B')
  start('dev');send('/familiar account dev default');wait(lambda:not bind.exists());stop();turn('dev','B');print(ui,'refresh isolated, expired blocked, rebound and cleared',flush=True)
 finally:stop()

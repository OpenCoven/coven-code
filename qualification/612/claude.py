import os,sys,time,subprocess,json,shlex,tempfile,pathlib,base64
P=pathlib.Path
binary=P(sys.argv[1]).resolve()
for ui in ['classic','next']:
 p=P(tempfile.mkdtemp(prefix='acct612-',dir='/tmp'));home=p/'engine';home.mkdir()
 (home/'settings.json').write_text('{"hasCompletedOnboarding":true}')
 (p/'familiars.toml').write_text('[[familiar]]\nid="dev"\nname="dev"\n[[familiar]]\nid="writer"\nname="writer"\n')
 cli=p/'claude';cli.write_text("""#!/usr/bin/env python3
import os,pathlib,sys,json
P=pathlib.Path;home=P(os.environ['HOME']);conf=P(os.environ.get('CLAUDE_CONFIG_DIR',str(home/'global')));args=sys.argv[1:]
if args[:2]==['auth','login']:
 if (home/'cancel').exists():sys.exit(130)
 (conf/'ok').touch();sys.exit(0)
if args[:2]==['auth','status']:
 print(json.dumps({'loggedIn':(conf/'ok').exists(),'authMethod':'claude.ai'}));sys.exit(0)
sys.stdin.readline()
identity=conf.parent.name if conf.name=='claude-config' else 'global'
with (home/'observed').open('a') as f:print(identity,file=f)
for row in [{'type':'system','subtype':'init','session_id':'synth-'+identity},{'type':'assistant','message':{'content':[{'type':'text','text':'fixture response '+identity}]}},{'type':'result','subtype':'success','result':'fixture response '+identity}]:print(json.dumps(row),flush=True)
""");cli.chmod(0o700)
 tag=p.name
 def tm(*args,check=True):return subprocess.run(['tmux','-L',tag,*args],capture_output=True,text=True,check=check)
 def pane():return tm('capture-pane','-p','-t','s',check=False).stdout
 def wait(f):
  for _ in range(120):
   if f():return
   time.sleep(.08)
  raise AssertionError((ui,str(p),pane()[-4500:]))
 def start(agent):
  env={'HOME':str(p),'COVEN_HOME':str(p),'COVEN_CODE_HOME':str(home),'COVEN_CODE_UI':ui,'COVEN_CODE_CLAUDE_BIN':str(cli),'COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1','PATH':os.environ['PATH'],'LANG':'C.UTF-8','TERM':'xterm-256color'}
  cmd='exec env -i '+shlex.join([k+'='+v for k,v in env.items()]+[str(binary),'--agent',agent])
  tm('new-session','-d','-x','140','-y','45','-s','s','-c',str(p),cmd);wait(lambda:any(x in pane() for x in ['shortcuts','preview','Coven v']));time.sleep(.45)
 def send(line):
  tm('send-keys','-l',line);time.sleep(.25);tm('send-keys','Enter');time.sleep(.45)
 def stop():tm('kill-server',check=False)
 def users():return json.loads((home/'accounts.json').read_text())['providers']['anthropic']['profiles']
 def count():return len((p/'observed').read_text().splitlines()) if (p/'observed').exists() else 0
 try:
  start('dev')
  for i in [1,2]:
   send('/account login anthropic --label work');wait(lambda:(home/'accounts.json').exists() and len(users())==i)
  assert set(users())=={'work','work-2'}
  (p/'cancel').touch();send('/account login anthropic --label canceled');assert len(users())==2;(p/'cancel').unlink()
  send('/account list --viable')
  if 'locally eligible' not in pane():tm('send-keys','Enter');time.sleep(.3)
  wait(lambda:all(x in pane() for x in ('anthropic/work','anthropic/work-2','locally eligible')))
  if 'esc' in pane().lower():tm('send-keys','Escape');time.sleep(.2)
  bind=home/'familiar-accounts'/'ZGV2.json'
  send('/familiar account dev anthropic/work');wait(bind.exists);assert json.loads(bind.read_text())['id']=='work'
  stop()
  for agent,expected in [('dev','work'),('writer','global')]:
   start(agent);n=count();send('identity check');wait(lambda:count()>n);assert (p/'observed').read_text().splitlines()[-1]==expected
   print(ui,agent,expected,'observed',expected,flush=True);stop()
  start('dev');send('/account list --viable')
  if 'last successful familiar request' not in pane():tm('send-keys','Enter');time.sleep(.3)
  wait(lambda:'last successful familiar request' in pane());print(ui,'verified account list and successful-request timestamp',flush=True)
  stop();start('dev');send('/familiar account dev default');wait(lambda:not bind.exists());stop()
  start('dev');n=count();send('identity check');wait(lambda:count()>n);assert (p/'observed').read_text().splitlines()[-1]=='global';print(ui,'cleared default observed global',flush=True)
 finally:stop()

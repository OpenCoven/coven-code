import json, os, pathlib, shlex, socket, subprocess, sys, threading, time
root=pathlib.Path(sys.argv[1]); binary=pathlib.Path(sys.argv[2]).resolve()
def run(mode,ui):
    p=root/(ui+'-'+mode); p.mkdir(); sock=p/'coven.sock'
    (p/'.coven-code').mkdir()
    (p/'.coven-code/settings.json').write_text(json.dumps(dict(hasCompletedOnboarding=True,daemonLedger=True)))
    (p/'familiars.toml').write_text('[[familiar]]\nid = "wisp"\n')
    listener=socket.socket(socket.AF_UNIX); listener.bind(str(sock)); listener.listen()
    hand=threading.Event()
    def serve(c):
        with c:
            raw=c.makefile('rb'); line=raw.readline().decode().strip(); n=0
            for h in raw:
                if not h.strip(): break
                if h.lower().startswith(b'content-length:'): n=int(h.split(b':')[1])
            data=raw.read(n)
            with (p/'requests').open('a') as f: print(line,data.decode(),file=f)
            if line.startswith('POST /api/v1/sessions '): hand.set()
            if line.startswith('POST /api/v1/sessions '):
                for _ in range(350):
                    if (p/'release').exists(): break
                    time.sleep(.01)
            body=b'{"session_id":"fixture-session","id":"fixture-session"}'
            msg=b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: '+str(len(body)).encode()+b'\r\nConnection: close\r\n\r\n'+body
            try:
                if mode=='trickle':
                    for b in msg: c.sendall(bytes([b]));time.sleep(.5)
                else: c.sendall(msg)
            except OSError: pass
    def accept():
        while True:
            try: c,_=listener.accept()
            except OSError: return
            threading.Thread(target=serve,args=(c,),daemon=True).start()
    threading.Thread(target=accept,daemon=True).start()
    tag='coven-580-'+str(os.getpid())+ui+mode
    def tm(*args,check=True):return subprocess.run(['tmux','-L',tag,*args],capture_output=True,text=True,check=check)
    def capture():return tm('capture-pane','-p','-t','s',check=False).stdout
    def wait(f,t):
        end=time.monotonic()+t
        while time.monotonic()<end:
            if f():return True
            time.sleep(.045)
        return False
    try:
        env=dict(PATH=os.environ['PATH'],HOME=str(p),COVEN_HOME=str(p),COVEN_CODE_UI=ui,COVEN_DAEMON_SOCKET=str(sock),COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1',TERM='xterm-256color',LANG='en_US.UTF-8')
        cli=p/'claude'
        cli.write_text("""#!/usr/bin/env python3
import json,sys,os,time,pathlib
sys.stdin.readline()
mark=pathlib.Path(os.environ['HOME'])
(mark/'active').touch()
for _ in range(300):
    if (mark/'finish').exists():break
    time.sleep(.1)
for x in [{"type":"system","subtype":"init","session_id":"fixture-seed"},{"type":"assistant","message":{"content":[{"type":"text","text":"frozen-seed-reply"}]}},{"type":"result","subtype":"success","result":"frozen-seed-reply"}]:print(json.dumps(x),flush=True)
"""); cli.chmod(0o700)
        env['COVEN_CODE_CLAUDE_BIN']=str(cli)
        tm('new-session','-d','-s','s','-x','120','-y','40','-c',str(p),'exec env -i '+shlex.join([k+'='+v for k,v in env.items()]+[str(binary)]))
        assert wait(lambda: 'shortcuts' in capture() or 'Coven v' in capture() or 'preview' in capture(),20),capture(); time.sleep(.45)
        (p/'boot').write_text(capture())
        tm('send-keys','-l','seed580');time.sleep(.25);tm('send-keys','Enter')
        assert wait(lambda:(p/'active').exists(),10),capture()
        tm('send-keys','-l','/update');time.sleep(.25);tm('send-keys','Enter')
        assert wait(lambda: 'Wait for the current response' in capture(),1.5),capture()
        (p/'finish').touch()
        assert wait(lambda:'frozen-seed-reply' in capture(),10),capture()
        assert 'Queued' not in capture(),capture()
        print(ui,'refused update and completed original turn',flush=True)
    finally:
        tm('kill-server',check=False);listener.close()
for ui in ['classic','next']:
    for mode in ['released']: run(mode,ui)

"""Reproduce #441 with the existing raw-PTY harness and synthetic history.

Usage: python3 long_history_pty.py BINARY OUTPUT_DIRECTORY [LINE_COUNT ...]

VirtualList tests pin exact row boundaries; this fixture names seeded answer
line counts, which exclude additional transcript headers and live output.
"""
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import termios
import time

spec = importlib.util.spec_from_file_location('perf', Path(__file__).with_name('perf_baseline.py'))
perf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(perf)

class Capture(perf.Session):
    def __init__(self, *args, **kwargs):
        self.raw = bytearray()
        super().__init__(*args, **kwargs)
    def read(self, timeout):
        data, at = super().read(timeout)
        self.raw.extend(data)
        return data, at
    def until(self, needles, timeout, start=0):
        deadline = time.monotonic()+timeout
        while time.monotonic()<deadline:
            visible = perf.ESCAPES.sub(b'',bytes(self.raw[start:]))
            if all(needle in visible for needle in needles):
                return True
            if self.proc.poll() is not None:
                self.read(.05)
                return False
            self.read(.1)
        return False
    def cleanup(self):
        if self.proc.poll() is None:
            os.killpg(self.proc.pid,signal.SIGKILL)
        os.close(self.master)
        self.proc.wait(timeout=5)


def journey(binary, output, width, height, lines):
    problems=[]
    with tempfile.TemporaryDirectory(prefix='coven-history-') as scratch:
        home=Path(scratch)/'home';project=Path(scratch)/'project'
        (home/'.coven-code').mkdir(parents=True);project.mkdir()
        (home/'.coven-code/settings.json').write_text('{"hasCompletedOnboarding":true}')
        session_id=perf.seed_session(str(home),str(project),lines)
        path=home/'.coven-code/sessions'/(session_id+'.json')
        saved=json.loads(path.read_text());saved['messages'][-1]['content']+='HISTORY-TAIL-441\n';path.write_text(json.dumps(saved))
        fake=perf.write_fake_claude(scratch,chunks=1,delay='0')
        session=Capture(binary,width,height,str(home),str(project),'classic',session_id,fake)
        code=None;restored=False
        try:
            if not session.until(perf.ready_needles('classic',True),20):
                problems.append('no usable resumed frame')
            else:
                if not session.until([b'HISTORY-TAIL-441'],5):
                    problems.append('resumed history tail is not visible')
                os.write(session.master,b'go\r')
                if not session.until([perf.STREAM_MARK],15):
                    problems.append('live answer tail is not visible')
                else:
                    # Force a full redraw after scrolling, so an unchanged
                    # prefix in a diffed frame cannot masquerade as missing text.
                    os.write(session.master,b'\x1b[5~'*5)
                    session.drain(.2)
                    start=len(session.raw)
                    perf.set_size(session.master,width,height+1)
                    os.kill(session.proc.pid,signal.SIGWINCH)
                    if not session.until([b'seeded line'],10,start):
                        problems.append('older semantic history not visible after scroll/resize')
                    os.write(session.master,b'\x1b[6~'*5)
                    session.drain(.2)
                    start=len(session.raw)
                    perf.set_size(session.master,width,height)
                    os.kill(session.proc.pid,signal.SIGWINCH)
                    if not session.until([perf.STREAM_MARK],10,start):
                        problems.append('return to live tail failed after scroll/resize')
            for _ in range(3):
                if session.proc.poll() is not None:break
                os.write(session.master,b'\x03');session.drain(.2)
            deadline=time.monotonic()+5
            while session.proc.poll() is None and time.monotonic()<deadline:session.read(.1)
            session.drain(.1)
            code=session.proc.poll()
            flags=termios.tcgetattr(session.master)[3]
            restored=bool(flags&termios.ECHO) and bool(flags&termios.ICANON)
            if code!=0:problems.append(f'ordinary exit status {code}')
            if not restored:problems.append('raw-mode flags not restored')
            if b'\x1b[?1049l' not in session.raw:problems.append('alternate screen not left')
            if b'\x1b[?25h' not in session.raw:problems.append('cursor not restored')
        finally:
            (output/f'{width}x{height}-{lines}.pty').write_bytes(session.raw)
            session.cleanup()
    return {'cells':f'{width}x{height}','seeded_answer_lines':lines,'exit_code':code,'raw_mode_restored':restored,'problems':problems}


def main():
    binary=str(Path(sys.argv[1]).resolve());output=Path(sys.argv[2]).resolve();output.mkdir(exist_ok=False)
    results=[]
    for width,height in [(80,24),(120,40)]:
        for lines in ([int(value) for value in sys.argv[3:]] or [65535,65536,100000]):
            result=journey(binary,output,width,height,lines);results.append(result)
            (output/'results.json').write_text(json.dumps({'binary':binary,'results':results},indent=2)+'\n')
            print(json.dumps(result),flush=True)
    return int(any(row['problems'] for row in results))

if __name__=='__main__':raise SystemExit(main())

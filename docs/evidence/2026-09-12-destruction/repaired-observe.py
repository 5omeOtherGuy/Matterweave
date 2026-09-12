import subprocess,time,pathlib,sys
root=pathlib.Path(__file__).parent
a=['adb','-s','192.168.178.93:5555']
def adb(*args):return subprocess.check_output(a+list(args),stderr=subprocess.DEVNULL)
paused=False;seen=set();deadline=time.monotonic()+120
while time.monotonic()<deadline:
 try: report=adb('exec-out','run-as','dev.matterweave.explorer','cat','files/destruction-check-report.txt').decode()
 except subprocess.CalledProcessError:time.sleep(.3);continue
 (root/'report-live.txt').write_text(report)
 for key,name in [('phase=0 source','source'),('phase=2 full-fracture-64','fractured'),('reset-cycle-10','reset-10')]:
  if key in report and name not in seen:
   if name == 'source': time.sleep(.7)
   (root/(name+'.png')).write_bytes(adb('exec-out','screencap','-p'));seen.add(name);print('capture',name,flush=True)
 if 'FAIL destruction' in report or report.splitlines()[-1].startswith('FAIL'):
  print(report[-3000:],flush=True);sys.exit(1)
 if 'PASS destruction:' in report:
  (root/'report.txt').write_text(report)
  print(report.splitlines()[-1],flush=True)
  summaries=[line for line in report.splitlines() if ' drawn=' in line]
  exact=len(summaries)==25 and all('drawn=60 frames' in line for line in summaries)
  exact=exact and sum(line.startswith('phase=4 lifecycle-recreate internal_recreations=') for line in report.splitlines())==1
  sys.exit(0 if exact and paused and 'platform_suspends=1' in report and 'platform_resumes=1' in report and 'internal_renderer_recreations=1' in report else 2)
 if 'phase=4 lifecycle-recreate' in report and not paused:
  adb('shell','input','keyevent','KEYCODE_HOME');time.sleep(3)
  adb('shell','am','start','-n','dev.matterweave.explorer/android.app.NativeActivity');paused=True
  print('real HOME/resume requested',flush=True)
 time.sleep(.4)
print('FAIL observer timeout; inspect report-live.txt',flush=True);sys.exit(3)

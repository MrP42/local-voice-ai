#!/usr/bin/env python3
"""Kill and resume the native import pipeline on a synthetic, multi-chunk video."""
import argparse, hashlib, json, pathlib, subprocess, time
p=argparse.ArgumentParser();p.add_argument('--device',required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
bundle='de.localvoice.prototype';fixture='TEST-video.mp4'
def sim(*args): return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
root=pathlib.Path(sim('get_app_container',a.device,bundle,'data'))/'Documents'
source=root/'MeetingFixtures'/fixture
expected=hashlib.sha256(source.read_bytes()).hexdigest()
sim('launch',a.device,bundle,'--meeting-import-probe',fixture)
def document():
 for path in (root/'Meetings').glob('*/meeting.json'):
  value=json.loads(path.read_text())
  if value['originalName']==fixture:return path,value
 return None,None
start=time.monotonic();checkpoint=None
while time.monotonic()-start<180:
 path,doc=document()
 phase=json.loads((root/'native-phase.json').read_text()).get('phase') if (root/'native-phase.json').exists() else None
 if doc and 0<doc['nextOffset']<doc['duration'] and phase=='transcribing':
  sim('terminate',a.device,bundle)
  path,checkpoint=document()
  break
 time.sleep(.2)
if checkpoint is None: raise RuntimeError('No mid-transfer checkpoint reached; inspect simulator')
assert checkpoint['nextOffset']>=30
sim('launch',a.device,bundle,'--meeting-import-probe',fixture)
start=time.monotonic()
while time.monotonic()-start<240:
 path,doc=document()
 if doc and doc.get('minutes') is not None:break
 time.sleep(.5)
else:raise RuntimeError('Deferred processing did not finish')
original=path.parent/('original.'+doc['originalExtension'])
assert hashlib.sha256(original.read_bytes()).hexdigest()==expected
assert doc['nextOffset']==doc['duration']
assert len({s['index'] for s in doc['segments']})==len(doc['segments'])
report={'fixture':fixture,'original_sha256':expected,'checkpoint_seconds':checkpoint['nextOffset'],'completed_seconds':doc['nextOffset'],'segments':len(doc['segments']),'timings_ms':doc.get('timings',{}),'minutes':doc['minutes'],'original_preserved':True,'resumed_without_duplicate_segments':True}
a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(a.output)

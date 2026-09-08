#!/usr/bin/env python3
"""Paired process-cold/warm iPhone trials; Watch restarts identically in both groups."""
import argparse,json,pathlib,subprocess,time,statistics,math
p=argparse.ArgumentParser();p.add_argument('--phone',required=True);p.add_argument('--watch',required=True);p.add_argument('--fixture',required=True,type=pathlib.Path);p.add_argument('--report',required=True,type=pathlib.Path);p.add_argument('--pairs',type=int,default=10);a=p.parse_args()
PB='de.localvoice.prototype';WB=PB+'.watchkitapp'
def sim(*args):return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
def root(d,b):return pathlib.Path(sim('get_app_container',d,b,'data'))
def rows(d,b):return {e['receipt']['sessionId']:e for f in (root(d,b)/'Library/Application Support/VoiceOutbox').glob('*/entry.json') for e in [json.loads(f.read_text())]}
if a.report.exists():raise SystemExit('Use a fresh report path; existing measurements are preserved')
fixture=root(a.watch,WB)/'Documents/fixture.m4a';original=fixture.read_bytes();results=[]
state={'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'definition':'Cold = restarted iPhone process; warm = following turn in same process. Watch restarts in both. OS caches are not flushed; native model contexts are still recreated per call.','pairs':a.pairs,'turns':results}
a.report.parent.mkdir(parents=True,exist_ok=True)
def persist():a.report.write_text(json.dumps(state,indent=2))
try:
 fixture.write_bytes(a.fixture.read_bytes())
 for pair in range(a.pairs):
  for group in ('process_cold','process_warm'):
   if group=='process_cold':sim('launch','--terminate-running-process',a.phone,PB,'--local-models','--stt-base')
   before=set(rows(a.watch,WB));sim('launch','--terminate-running-process',a.watch,WB,'--fixture-capture');started=time.monotonic()
   identifier=None
   while time.monotonic()-started<180:
    w=rows(a.watch,WB);p=rows(a.phone,PB);new=set(w)-before
    if len(new)==1:identifier=next(iter(new))
    if identifier and identifier in p and w[identifier].get('replyReceipt') and p[identifier].get('replyAcknowledged') and 'tts_e2e_ms' in w[identifier]['timings']:
     assert p[identifier]['digest']==w[identifier]['digest']
     assert p[identifier]['reply']==w[identifier]['reply'] and p[identifier]['transcript']==w[identifier]['transcript']
     results.append({'pair':pair,'group':group,'id':identifier,'phoneTimings':p[identifier]['timings'],'watchTimings':w[identifier]['timings'],'audited':True})
     persist();print(group,pair+1,round(w[identifier]['timings']['tts_e2e_ms']),flush=True);break
    time.sleep(.5)
   else:state['pending']={'pair':pair,'group':group,'id':identifier};persist();raise RuntimeError('Measurement did not finish')
 state['statistics']={}
 for group in ('process_cold','process_warm'):
  stats={}
  for side,key in [('phone','stt_ms'),('phone','generation_ms'),('watch','transfer_roundtrip_ms'),('watch','reply_e2e_ms'),('watch','tts_e2e_ms')]:
   values=[t[side+'Timings'][key] for t in results if t['group']==group and key in t[side+'Timings']]
   stats[key]={'n':len(values),'median':statistics.median(values),'p95':sorted(values)[math.ceil(.95*len(values))-1],'max':max(values)}
  state['statistics'][group]=stats
 state['passed']=True;persist()
finally:fixture.write_bytes(original)

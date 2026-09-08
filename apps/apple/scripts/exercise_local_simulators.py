#!/usr/bin/env python3
"""100 synthetic local-model turns with explicit simulator lifecycle faults.
Only use task-owned simulator IDs. Never erase app data. Report can be resumed.
"""
import argparse, json, pathlib, subprocess, time, statistics
p=argparse.ArgumentParser(); p.add_argument('--phone',required=True);p.add_argument('--watch',required=True);p.add_argument('--report',required=True);p.add_argument('--resume',action='store_true');a=p.parse_args()
report=pathlib.Path(a.report)
PB='de.localvoice.prototype';WB=PB+'.watchkitapp'
def sim(*args,check=True): return subprocess.run(['xcrun','simctl',*args],text=True,capture_output=True,check=check).stdout.strip()
def root(d,b): return pathlib.Path(sim('get_app_container',d,b,'data'))/'Library/Application Support/VoiceOutbox'
def entries(d,b): return {e['receipt']['sessionId']:e for f in root(d,b).glob('*/entry.json') for e in [json.loads(f.read_text())]}
def persist():
 temp=report.with_suffix('.tmp');temp.write_text(json.dumps(state,indent=2));temp.replace(report)
if a.resume:
 state=json.loads(report.read_text())
else:
 if report.exists(): raise SystemExit('Report exists; use --resume instead of seeding again')
 before=set(entries(a.watch,WB))
 state={'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'startedAt':time.time(),'initialIds':sorted(before),'ids':[],'milestones':[],'phase':'seeding'}
 report.parent.mkdir(parents=True,exist_ok=True);persist()
 sim('launch','--terminate-running-process',a.phone,PB,'--local-models')
 sim('launch','--terminate-running-process',a.watch,WB,'--fixture-capture','--fixture-count','100')
 for _ in range(60):
  ids=set(entries(a.watch,WB))-before
  if len(ids)==100: state['ids']=sorted(ids);break
  time.sleep(1)
 else: raise RuntimeError('Expected exactly 100 new persisted captures')
 state['phase']='running';persist()
ids=set(state['ids'])
while True:
 try:
  watch=entries(a.watch,WB); phone=entries(a.phone,PB)
  if not ids.issubset(watch): raise RuntimeError('A confirmed Watch capture is missing')
  accepted=len(ids & phone.keys());answered=sum(phone[i].get('reply') is not None for i in ids & phone.keys())
  delivered=sum(watch[i].get('replyReceipt') is not None for i in ids)
  state['counts']={'watchRetained':len(ids),'phoneAccepted':accepted,'phoneAnswered':answered,'watchReplyReceipts':delivered}
  state['elapsedSeconds']=time.time()-state['startedAt'];persist()
  print(json.dumps(state['counts']),flush=True)
  milestones=state['milestones']
  if accepted>=5 and 'phone_background' not in milestones:
   sim('launch',a.phone,'com.apple.mobilesafari');time.sleep(8)
   state['backgroundSnapshot']={'accepted':len(ids & entries(a.phone,PB).keys())}
   sim('launch',a.phone,PB);milestones.append('phone_background');persist()
  if accepted>=15 and 'watch_restart' not in milestones:
   sim('launch','--terminate-running-process',a.watch,WB);milestones.append('watch_restart');persist()
  if accepted>=25 and 'phone_restart' not in milestones:
   sim('launch','--terminate-running-process',a.phone,PB,'--local-models');milestones.append('phone_restart');persist()
  if accepted>=40 and 'phone_disconnect' not in milestones:
   sim('shutdown',a.phone);time.sleep(8);sim('boot',a.phone);sim('bootstatus',a.phone,'-b')
   sim('launch',a.phone,PB,'--local-models');sim('launch','--terminate-running-process',a.watch,WB)
   milestones.append('phone_disconnect');persist()
  if delivered==100 and answered==100:
   state['phase']='complete';state['finishedAt']=time.time()
   state['turns']=[{'id':i,'digestMatches':phone[i]['digest']==watch[i]['digest'],'phoneTimings':phone[i].get('timings',{}),'watchTimings':watch[i].get('timings',{}),'hasTranscript':bool(phone[i].get('transcript')),'replyAcknowledged':phone[i].get('replyAcknowledged',False)} for i in sorted(ids)]
   if not all(t['digestMatches'] and t['hasTranscript'] for t in state['turns']): raise RuntimeError('Missing transcript or changed audio digest')
   persist();print('COMPLETE: 100 locally transcribed and answered captures with Watch receipts',flush=True);break
  if state['elapsedSeconds']>7200: raise RuntimeError('Observation budget expired; preserve report and resume after diagnosis')
  time.sleep(5)
 except Exception as error:
  state['phase']='observation_error';state['error']=str(error);persist();raise

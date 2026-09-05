#!/usr/bin/env python3
"""Synthetic P2 quality cases through WatchConnectivity and actual iPhone CPU providers."""
import argparse,json,pathlib,subprocess,time,hashlib
p=argparse.ArgumentParser();p.add_argument('--phone',required=True);p.add_argument('--watch',required=True);p.add_argument('--fixtures',required=True,type=pathlib.Path);p.add_argument('--report',required=True,type=pathlib.Path);p.add_argument('--small',action='store_true');a=p.parse_args()
PB='de.localvoice.prototype';WB=PB+'.watchkitapp'
def sim(*args):return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
def root(device,bundle):return pathlib.Path(sim('get_app_container',device,bundle,'data'))
def entries(device,bundle):
 return {e['receipt']['sessionId']:e for f in (root(device,bundle)/'Library/Application Support/VoiceOutbox').glob('*/entry.json') for e in [json.loads(f.read_text())]}
cases=json.loads((pathlib.Path(__file__).resolve().parents[1]/'tests/german-p2-cases.json').read_text())
if a.report.exists():raise SystemExit('Report exists; preserve it and choose a new run path')
state={'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'model':'small' if a.small else 'base','cases':[],'startedAt':time.time()}
a.report.parent.mkdir(parents=True,exist_ok=True)
def persist():a.report.write_text(json.dumps(state,indent=2,ensure_ascii=False))
fixture=root(a.watch,WB)/'Documents/fixture.m4a';original=fixture.read_bytes()
sim('launch','--terminate-running-process',a.phone,PB,'--local-models','--stt-small' if a.small else '--stt-base')
try:
 for case in cases:
  before=set(entries(a.watch,WB));audio=(a.fixtures/(case['id']+'.m4a')).read_bytes();fixture.write_bytes(audio)
  started=time.monotonic();sim('launch','--terminate-running-process',a.watch,WB,'--fixture-capture')
  identifier=None
  for _ in range(240):
   watch=entries(a.watch,WB);phone=entries(a.phone,PB);new=set(watch)-before
   if len(new)==1:identifier=next(iter(new))
   if identifier and identifier in phone:
    w=watch[identifier];f=phone[identifier];phase=f.get('job',{}).get('phase')
    if (w.get('replyReceipt') and f.get('replyAcknowledged')) or (phase=='failed' and w['state']!='saved'):
     assert f['digest']==w['digest']==hashlib.sha256(audio).hexdigest()
     text=f.get('transcript','');answer=f.get('reply','')
     capability='Ich kann hier antworten und Notizen speichern. Eine externe Aktion habe ich nicht ausgeführt.'
     result={'case':case['id'],'id':identifier,'sourceText':case['text'],'inputSHA256':hashlib.sha256(audio).hexdigest(),
       'transcript':text,'reply':answer,'job':f.get('job'),'watchState':w['state'],
       'keywordChecks':{k:k.casefold() in text.casefold() for k in case['checks']},
       'phoneTimings':f.get('timings',{}),'watchTimings':w.get('timings',{}),
       'elapsedSeconds':time.monotonic()-started,'audioPreserved':True}
     if case.get('capabilityExpected'):result['capabilityBoundaryPassed']=answer==capability
     if case.get('replyAny'):result['answerCheckPassed']=any(v in answer.casefold() for v in case['replyAny'])
     if case.get('expectNoSpeech'):result['noSpeechPassed']=not answer and f.get('job',{}).get('failure')=='noSpeech'
     state['cases'].append(result);persist();print(case['id'],{k:v for k,v in result.items() if k.endswith('Passed')},flush=True);break
   time.sleep(1)
  else:
   state['pending']={'case':case['id'],'id':identifier};persist();raise RuntimeError('Case did not finish: '+case['id'])
 state['finishedAt']=time.time();state['completed']=True;persist()
finally:fixture.write_bytes(original)

#!/usr/bin/env python3
"""Task-owned simulator lifecycle probes; actual interruption is explicitly injected."""
import argparse,hashlib,json,pathlib,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--phone',required=True);p.add_argument('--watch',required=True);p.add_argument('--report',type=pathlib.Path,required=True);p.add_argument('--case',choices=['background','locked','force-quit-resume','denied','interruption','duplicate'],required=True);a=p.parse_args()
PB='de.localvoice.prototype';WB=PB+'.watchkitapp'
def sim(*args):return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
def root(d,b):return pathlib.Path(sim('get_app_container',d,b,'data'))
pr=root(a.phone,PB);wr=root(a.watch,WB)
def rows(r):return {e['receipt']['sessionId']:e for f in (r/'Library/Application Support/VoiceOutbox').glob('*/entry.json') for e in [json.loads(f.read_text())]}
def wait(f,timeout=120):
 start=time.monotonic()
 while time.monotonic()-start<timeout:
  value=f()
  if value:return value
  time.sleep(.5)
 raise RuntimeError('Condition timed out')
def control(name):
 log=pathlib.Path('/tmp/local-voice-p2-'+name+'.log')
 with log.open('w') as stream:subprocess.run(['xcodebuild','test','-project','apps/apple/LocalVoice.xcodeproj','-scheme','SimulatorControlsUITests','-destination','platform=macOS','-derivedDataPath','apps/apple/DerivedData','-only-testing:SimulatorControlsUITests/SimulatorControlsTests/'+name],stdout=stream,stderr=subprocess.STDOUT,check=True)
def snapshot(r):return {i:(e['digest'],e.get('replyMessageId'),e.get('replyReceipt')) for i,e in rows(r).items()}
if a.report.exists():raise SystemExit('Preserve existing evidence; use a new path')
result={'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'case':a.case,'startedAt':time.time()}
if a.case in ('background','locked','force-quit-resume'):
 if a.case!='force-quit-resume':
  sim('launch','--terminate-running-process',a.phone,PB,'--local-models','--stt-base')
  time.sleep(2)
  if a.case=='background':sim('launch',a.phone,'com.apple.mobilesafari')
  else:control('testLockPhone')
 before=set(rows(wr));sim('launch','--terminate-running-process',a.watch,WB,'--fixture-capture')
 identifier=wait(lambda:next(iter(set(rows(wr))-before),None));time.sleep(10)
 w=rows(wr)[identifier];phone=rows(pr).get(identifier)
 assert not w.get('reply') and not (phone or {}).get('reply')
 result.update(id=identifier,deferred=True,watchState=w['state'],phoneState=(phone or {}).get('state'),observationSeconds=10)
 if a.case=='locked':control('testRaisePhone')
 sim('launch',a.phone,PB,'--local-models','--stt-base');sim('launch','--terminate-running-process',a.watch,WB)
 wait(lambda:rows(wr)[identifier].get('replyReceipt') and rows(pr).get(identifier,{}).get('replyAcknowledged'),180)
 final=rows(pr)[identifier];assert final['digest']==w['digest']==hashlib.sha256((wr/'Library/Application Support/VoiceOutbox'/identifier/'audio.m4a').read_bytes()).hexdigest()
 result.update(resumedSameIdentity=True,audioPreserved=True,phoneTimings=final['timings'])
elif a.case=='denied':
 result['devices']=[]
 for device,bundle,r in [(a.phone,PB,pr),(a.watch,WB,wr)]:
  before=set(rows(r));event=r/'Documents/last-event.txt'
  if event.exists():event.unlink()
  try:
   sim('privacy',device,'revoke','microphone',bundle)
   sim('launch','--terminate-running-process',device,bundle,'--record-probe')
   wait(lambda:event.exists() and event.read_text()=='microphone_denied',30)
   assert set(rows(r))==before
   result['devices'].append({'bundle':bundle,'denialShown':True,'noCaptureFalselyConfirmed':True})
  finally:sim('privacy',device,'grant','microphone',bundle)
elif a.case=='interruption':
 before=snapshot(wr);event=wr/'Documents/last-event.txt'
 if event.exists():event.unlink()
 sim('launch','--terminate-running-process',a.watch,WB,'--interruption-notification')
 wait(lambda:event.exists() and event.read_text()=='audio_interrupted',30)
 assert snapshot(wr)==before
 result.update(injection='AVAudioSession interruption began after actual AVSpeechSynthesizer didStart',historyPreserved=True,physicalPhoneCallTest=False)
elif a.case=='duplicate':
 before=snapshot(pr);wbefore=snapshot(wr);event=wr/'Documents/last-event.txt'
 if event.exists():event.unlink()
 sim('launch',a.phone,PB,'--local-models');sim('launch','--terminate-running-process',a.watch,WB,'--replay-capture')
 wait(lambda:event.exists() and event.read_text()=='duplicate_replayed',30)
 assert snapshot(pr)==before and snapshot(wr)==wbefore
 result.update(stableIdentities=True,noNewHistoryEntry=True)
result['passed']=True;result['finishedAt']=time.time();a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps(result,indent=2));print('PASS:',a.case,flush=True)

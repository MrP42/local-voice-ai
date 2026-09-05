#!/usr/bin/env python3
"""Verify a real simulator recorder is saved across the native Wrist Down control."""
import json,pathlib,subprocess,time,hashlib
P='E4D1D247-E548-4E89-83BA-A2B3FCB50853';W='41324A06-70C6-4C60-AE7F-B62EECB30F91';PB='de.localvoice.prototype';WB=PB+'.watchkitapp'
def sim(*args):return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
def root(d,b):return pathlib.Path(sim('get_app_container',d,b,'data'))/'Library/Application Support/VoiceOutbox'
wr=root(W,WB);pr=root(P,PB)
def rows(r):return {e['receipt']['sessionId']:e for f in r.glob('*/entry.json') for e in [json.loads(f.read_text())]}
report=pathlib.Path('docs/apple-evidence/p2/results/wrist.json')
if report.exists():raise SystemExit('Preserve existing wrist evidence')
before=set(rows(wr));sim('launch',P,PB,'--local-models')
subprocess.run(['python3','apps/apple/scripts/test_wrist_simulator.py'],check=True)
for _ in range(60):
 new=set(rows(wr))-before
 if len(new)==1:break
 time.sleep(.5)
else:raise RuntimeError('No single saved recorder capture after Wrist Down')
i=next(iter(new));w=rows(wr)[i]
assert 'record_feedback_ms' in w['timings']
result={'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'id':i,'stateWhileWristDown':w['state'],'recordingTimings':w['timings'],'nativeControlLog':'local-voice-wrist-down-final.log'}
with open('/tmp/local-voice-p2-raise-wrist.log','w') as log:subprocess.run(['xcodebuild','test','-project','apps/apple/LocalVoice.xcodeproj','-scheme','SimulatorControlsUITests','-destination','platform=macOS','-derivedDataPath','apps/apple/DerivedData','-only-testing:SimulatorControlsUITests/SimulatorControlsTests/testRaiseWrist'],stdout=log,stderr=subprocess.STDOUT,check=True)
sim('launch',W,WB)
for _ in range(120):
 phone=rows(pr).get(i);watch=rows(wr)[i]
 if phone and watch['state']!='saved':break
 time.sleep(.5)
else:raise RuntimeError('Retained recording not accepted after Wrist Up')
for r in (pr,wr):assert hashlib.sha256((r/i/'audio.m4a').read_bytes()).hexdigest()==w['digest']
result.update(passed=True,sameCaptureAcceptedByPhone=True,originalsVerified=True,scope='Simulator Wrist Down and recorder persistence; no physical wrist or microphone quality claim')
report.write_text(json.dumps(result,indent=2));print('PASS: recorded audio persisted across Wrist Down and accepted by iPhone')

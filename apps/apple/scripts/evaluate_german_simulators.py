#!/usr/bin/env python3
"""Evaluate regenerated desktop German cases through the real paired app path."""
import argparse, pathlib, json, subprocess, shutil, time, hashlib, re
p=argparse.ArgumentParser();p.add_argument('--phone',required=True);p.add_argument('--watch',required=True);p.add_argument('--fixtures',required=True,type=pathlib.Path);p.add_argument('--report',required=True,type=pathlib.Path);a=p.parse_args()
PB='de.localvoice.prototype';WB=PB+'.watchkitapp'
def sim(*args): return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
def data(d,b): return pathlib.Path(sim('get_app_container',d,b,'data'))
def rows(d,b): return {e['receipt']['sessionId']:e for f in (data(d,b)/'Library/Application Support/VoiceOutbox').glob('*/entry.json') for e in [json.loads(f.read_text())]}
results=[]
cases=json.loads((pathlib.Path(__file__).resolve().parents[1]/'tests/german-cases.json').read_text())
fixture=data(a.watch,WB)/'Documents/fixture.m4a'
original=fixture.read_bytes()
sim('launch','--terminate-running-process',a.phone,PB,'--local-models')
try:
 for case in cases:
  before=set(rows(a.watch,WB));source=a.fixtures/(case['id']+'.m4a');shutil.copy(source,fixture)
  sim('launch','--terminate-running-process',a.watch,WB,'--fixture-capture')
  started=time.monotonic();capture=None
  while time.monotonic()-started<180:
   current=rows(a.watch,WB);new=set(current)-before
   if len(new)==1: capture=current[next(iter(new))]
   if capture and capture.get('replyReceipt'):
    phone=rows(a.phone,PB)[capture['receipt']['sessionId']]
    transcript=phone.get('transcript','')
    result={'case':case['id'],'inputSHA256':hashlib.sha256(source.read_bytes()).hexdigest(),'sourceText':case['text'],'transcript':transcript,'reply':phone.get('reply'),'keywordChecks':{k:k.casefold() in transcript.casefold() for k in case['checks']},'phoneTimings':phone.get('timings'),'watchTimings':capture.get('timings'),'elapsedSeconds':time.monotonic()-started}
    results.append(result);a.report.write_text(json.dumps(results,indent=2,ensure_ascii=False));print(case['id'],result['keywordChecks'],flush=True);break
   time.sleep(1)
  else: raise RuntimeError('Case did not complete: '+case['id'])
finally: fixture.write_bytes(original)

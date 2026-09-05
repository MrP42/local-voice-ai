#!/usr/bin/env python3
"""Test verified atomic model installation with valid and deliberately corrupted official weights."""
import argparse,hashlib,json,pathlib,shutil,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--phone',required=True);p.add_argument('--report',required=True,type=pathlib.Path);a=p.parse_args();bundle='de.localvoice.prototype'
def sim(*args):return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
doc=pathlib.Path(sim('get_app_container',a.phone,bundle,'data'))/'Documents';base=doc/'Models/ggml-base.bin';bad=doc/'invalid-model-probe.bin';event=doc/'last-event.txt'
def digest(path):
 h=hashlib.sha256()
 with path.open('rb') as f:
  while block:=f.read(1024*1024):h.update(block)
 return h.hexdigest()
original=digest(base);results=[]
try:
 shutil.copyfile(base,bad)
 with bad.open('r+b') as f:
  byte=f.read(1);f.seek(0);f.write(bytes([byte[0]^255]))
 for invalid in (True,False):
  if event.exists():event.unlink()
  inode=base.stat().st_ino
  sim('launch','--terminate-running-process',a.phone,bundle,'--model-import-probe',*(['--invalid-model'] if invalid else []))
  expected='model_import_failed' if invalid else 'model_import_verified'
  for _ in range(120):
   if event.exists() and event.read_text()==expected:break
   time.sleep(.5)
  else:raise RuntimeError('Model import probe did not produce expected outcome')
  assert digest(base)==original
  if invalid:assert base.stat().st_ino==inode
  else:assert base.stat().st_ino!=inode
  assert not list((doc/'Models').glob('.install-*'))
  results.append({'invalidInput':invalid,'outcome':expected,'originalHashPreserved':True,'temporaryCopiesRemoved':True})
 a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps({'passed':True,'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'cases':results},indent=2))
 print('PASS: invalid model rejected, valid model atomically replaced after verification')
finally:
 if bad.exists():bad.unlink()

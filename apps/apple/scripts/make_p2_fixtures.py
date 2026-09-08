#!/usr/bin/env python3
"""Deterministic synthetic speech, digital silence and noise; never records a microphone."""
import argparse,json,pathlib,subprocess,wave,struct,random
p=argparse.ArgumentParser();p.add_argument('output',type=pathlib.Path);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
cases=json.loads((pathlib.Path(__file__).resolve().parents[1]/'tests/german-p2-cases.json').read_text())
for case in cases:
 target=a.output/(case['id']+'.m4a')
 if target.exists():continue
 if case.get('signal'):
  source=a.output/(case['id']+'.wav');rng=random.Random(42)
  with wave.open(str(source),'wb') as w:
   w.setparams((1,2,16000,0,'NONE','not compressed'))
   samples=[0 if case['signal']=='silence' else max(-32768,min(32767,int(rng.gauss(0,.035)*32767))) for _ in range(16000*4)]
   w.writeframes(struct.pack('<'+'h'*len(samples),*samples))
 else:
  source=a.output/(case['id']+'.aiff')
  subprocess.run(['say','-v','Anna','-o',str(source),case['text']],check=True)
 subprocess.run(['afconvert','-f','m4af','-d','aac','-b','32000',str(source),str(target)],check=True)
 print(case['id'],flush=True)

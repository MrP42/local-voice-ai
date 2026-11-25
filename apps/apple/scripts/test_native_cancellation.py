#!/usr/bin/env python3
"""CPU-Spracherkennung, STT und Generierung abbrechen, dann fortsetzen (Base oder Turbo)."""
import argparse,json,pathlib,subprocess,time,sys
# Ohne Simulator: statische Erkennungs-, Sprach- und Deadline-Grenzprüfungen.
if sys.argv[1:] == ['--static-check']:
 import unittest
 from test_whisper_contracts import WhisperContracts
 result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(WhisperContracts))
 sys.exit(0 if result.wasSuccessful() else 1)
p=argparse.ArgumentParser()
p.add_argument('--phone',required=True)
p.add_argument('--report',required=True,type=pathlib.Path)
p.add_argument('--stt-model',choices=('base','turbo'),default='base')
a=p.parse_args()
bundle='de.localvoice.prototype'
def sim(*args):return subprocess.check_output(['xcrun','simctl',*args],text=True).strip()
def documents():return pathlib.Path(sim('get_app_container',a.phone,bundle,'data'))/'Documents'
def entries():
 root=documents().parent/'Library/Application Support/VoiceOutbox'
 return {e['receipt']['sessionId']:e for f in root.glob('*/entry.json') for e in [json.loads(f.read_text())]}
def launch(*args):
 sim('launch','--terminate-running-process',a.phone,bundle,'--local-models',
     '--stt-'+a.stt_model,'--stt-language-auto',*args)
launch()
for _ in range(60):
 pending=[e for e in entries().values() if e.get('reply') is None and e.get('job',{}).get('phase') not in ('failed','cancelled') and e.get('job',{}).get('attempts',0)<3]
 if not pending:break
 time.sleep(1)
else:raise RuntimeError('Existing processing has not drained; do not cancel an unrelated turn')
before=set(entries()); results=[]; target=None
for phase in ('detecting_language','transcribing','generating'):
 probe=documents()/'cancellation-probe.json'
 if probe.exists():probe.unlink()
 args=['--cancel-inference-probe']
 if target:args+=['--retry-job',target]
 else:args+=['--fixture-capture']
 if phase=='detecting_language':args+=['--cancel-language-detection']
 if phase=='generating':args+=['--cancel-generating']
 launch(*args)
 for _ in range(120):
  if probe.exists():break
  time.sleep(1)
 else:raise RuntimeError('Cancellation probe did not complete; preserve existing captures')
 result=json.loads(probe.read_text());print(json.dumps(result),flush=True)
 assert result['phase']==phase and result['cancelled'] and result['nativeStoppedAfterRequest'] and not result['hasReply']
 if target:assert result['id']==target
 else:
  target=result['id'];assert target not in before
  assert set(entries())-before=={target}
 assert result['hasTranscript']==(phase=='generating')
 results.append(result)
saved=entries()[target];launch('--retry-job',target)
for _ in range(120):
 final=entries()[target]
 if final.get('reply'):break
 time.sleep(1)
else:raise RuntimeError('Resumed capture did not complete')
assert final['digest']==saved['digest'] and final['job']['phase']=='completed'
assert final['transcript']==saved['transcript'] and final['timings']['stt_ms']==saved['timings']['stt_ms']
a.report.parent.mkdir(parents=True,exist_ok=True)
a.report.write_text(json.dumps({
 'passed':True,
 'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
 'testedWorkingChanges':True,'id':target,'probes':results,'sttModel':a.stt_model,
 'backend':'simulator-cpu','resumedWithoutRetranscription':True,'audioPreserved':True
},indent=2))
print('PASS: actual native STT/generation cancelled; same recording completed after explicit resume',flush=True)

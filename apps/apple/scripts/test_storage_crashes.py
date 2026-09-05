#!/usr/bin/env python3
"""Abrupt exits at durable storage boundaries using only disposable synthetic data."""
import argparse,json,pathlib,subprocess,tempfile,time
p=argparse.ArgumentParser();p.add_argument('--report',required=True,type=pathlib.Path);a=p.parse_args()
base=pathlib.Path(__file__).resolve().parents[1]
probe=base/'VoiceCore/.build/debug/VoiceCrashProbe'
def run(mode,root,checkpoint=None):
 args=[str(probe),mode,str(root)]+([checkpoint] if checkpoint else [])
 return subprocess.run(args,text=True,capture_output=True,timeout=15)
def success(mode,root):
 result=run(mode,root)
 if result.returncode:raise RuntimeError(result.stderr)
 return json.loads(result.stdout)
results=[]
for mode,points in [('capture',['beforeWrite','fileSynced','captureMetadataSynced','captureRenamed','captureCommitted']),('reply',['beforeWrite','fileSynced','replyCommitted']),('ack',['beforeWrite','fileSynced','replyAckCommitted']),('job',['jobStarted','transcriptCommitted','generatedReplyCommitted'])]:
 for point in points:
  with tempfile.TemporaryDirectory(prefix='local-voice-p2-crash-') as path:
   root=pathlib.Path(path)
   killed=run(mode,root,point)
   assert killed.returncode==77,(mode,point,killed.returncode,killed.stderr)
   before=success('inspect',root)
   after=success(mode,root)
   repeated=success(mode,root)
   assert after['audioMatches'] and repeated['audioMatches']
   assert len(after['entries'])==len(repeated['entries'])==1
   e=after['entries'][0]; r=repeated['entries'][0]
   assert e['receipt']==r['receipt']
   if before['entries']:assert before['entries'][0]['receipt']==e['receipt']
   if mode=='reply':assert e['replyReceipt']==r['replyReceipt'] and e['reply']=='Erhalten.'
   if mode=='job':assert e['transcript']==r['transcript']=='Ein dauerhaftes Transkript.' and e['replyMessageId']==r['replyMessageId']
   if mode=='ack':assert e['replyAcknowledged'] is True and e['replyMessageId']==r['replyMessageId']
   results.append({'mode':mode,'checkpoint':point,'exitCode':77,'passed':True,'recoverableImmediately':len(before['entries']),'retainedUnidentifiedStages':len(after['issues'])})
a.report.parent.mkdir(parents=True,exist_ok=True)
a.report.write_text(json.dumps({'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'testedWorkingChanges':True,'finishedAt':time.time(),'cases':results,'passed':True,'limit':'Process exits, not physical power loss; injected ENOSPC covered separately by core tests'},indent=2))
print('PASS:',len(results),'abrupt process exits; originals and stable acknowledgements preserved')

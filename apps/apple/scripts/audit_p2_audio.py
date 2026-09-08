#!/usr/bin/env python3
"""Verify actual original bytes and acknowledged response identities for a finished P2 cohort."""
import argparse,hashlib,json,pathlib,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--phone',required=True);p.add_argument('--watch',required=True);p.add_argument('--report',required=True,type=pathlib.Path);a=p.parse_args()
d=json.loads(a.report.read_text());ids=d['ids'];assert len(ids)==len(set(ids))==100
roots=[]
for device,bundle in [(a.phone,'de.localvoice.prototype'),(a.watch,'de.localvoice.prototype.watchkitapp')]:
 roots.append(pathlib.Path(subprocess.check_output(['xcrun','simctl','get_app_container',device,bundle,'data'],text=True).strip())/'Library/Application Support/VoiceOutbox')
for identifier in ids:
 entries=[]
 for root in roots:
  folder=root/identifier;e=json.loads((folder/'entry.json').read_text());entries.append(e)
  assert hashlib.sha256((folder/'audio.m4a').read_bytes()).hexdigest()==e['digest'],identifier
 phone,watch=entries
 assert phone['digest']==watch['digest'] and phone['transcript']==watch['transcript'] and phone['reply']==watch['reply']
 assert phone['replyAcknowledged'] and watch['replyReceipt']['messageId']==phone['replyMessageId']
 assert phone['job']['phase']=='completed' and phone['state']==watch['state']=='answered'
d['actualAudioAudit']={'passed':True,'originalFilesRead':200,'count':100,'at':time.time(),'checks':['actual SHA-256 on both devices','stable response identity','durable acknowledgement','completed persistent job','same transcript and answer']}
a.report.write_text(json.dumps(d,indent=2));print('PASS: all 200 actual original audio files and 100 completed jobs verified')

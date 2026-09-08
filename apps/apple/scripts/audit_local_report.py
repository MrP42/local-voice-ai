#!/usr/bin/env python3
"""Fresh requirement-level audit of an already executed native-model turn set."""
import argparse,json,pathlib,subprocess
p=argparse.ArgumentParser();p.add_argument('--phone',required=True);p.add_argument('--watch',required=True);p.add_argument('--report',required=True,type=pathlib.Path);a=p.parse_args()
report=json.loads(a.report.read_text())
def entries(d,b):
 root=pathlib.Path(subprocess.check_output(['xcrun','simctl','get_app_container',d,b,'data'],text=True).strip())/'Library/Application Support/VoiceOutbox'
 return {e['receipt']['sessionId']:e for f in root.glob('*/entry.json') for e in [json.loads(f.read_text())]}
phone=entries(a.phone,'de.localvoice.prototype');watch=entries(a.watch,'de.localvoice.prototype.watchkitapp')
ids=report['ids'];assert len(ids)==100 and len(set(ids))==100
for identifier in ids:
 p=phone[identifier];w=watch[identifier]
 assert p['digest']==w['digest'],identifier
 assert p['transcript'] and p['transcript']==w['transcript'],identifier
 assert p['reply'] and p['reply']==w['reply'],identifier
 assert p['timings'].get('stt_ms',0)>0 and p['timings'].get('generation_ms',0)>0,identifier
 assert p.get('replyAcknowledged') is True,identifier
 assert w['replyReceipt']['messageId']==p['replyMessageId'],identifier
 assert p['state']==w['state']=='answered',identifier
report['finalAudit']={'passed':True,'count':100,'checks':['original audio digest','transcript preserved on both sides','reply preserved on both sides','STT timing present','generation timing present','durable response acknowledgement','stable response identity','answered state']}
a.report.write_text(json.dumps(report,indent=2));print('PASS: all 100 local-model turns satisfy every final audit assertion')

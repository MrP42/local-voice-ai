#!/usr/bin/env python3
"""Read persisted simulator evidence without logging conversation text."""
import argparse, json, pathlib, subprocess, statistics
p=argparse.ArgumentParser(); p.add_argument('--phone',required=True); p.add_argument('--watch',required=True); a=p.parse_args()
def inspect(device,bundle):
    root=pathlib.Path(subprocess.check_output(['xcrun','simctl','get_app_container',device,bundle,'data'],text=True).strip())
    outbox=root/'Library/Application Support/VoiceOutbox'
    entries=[json.loads(f.read_text()) for f in outbox.glob('*/entry.json')]
    timings={}
    for e in entries:
        for key,value in e.get('timings',{}).items(): timings.setdefault(key,[]).append(value)
    result={'count':len(entries),'states':{state:sum(e['state']==state for e in entries) for state in ('saved','accepted','deferred','answered')},'ids':[e['receipt']['sessionId'] for e in entries], 'timings_ms':{k:{'n':len(v),'median':statistics.median(v),'p95':sorted(v)[max(0,__import__('math').ceil(.95*len(v))-1)],'max':max(v)} for k,v in timings.items()}}
    capabilities=root/'Documents/capabilities.json'
    if capabilities.exists(): result['capabilities']=json.loads(capabilities.read_text())
    return result
phone=inspect(a.phone,'de.localvoice.prototype');watch=inspect(a.watch,'de.localvoice.prototype.watchkitapp')
print(json.dumps({'phone':phone,'watch':watch,'watch_captures_missing_on_phone':sorted(set(watch['ids'])-set(phone['ids']))},indent=2))

#!/usr/bin/env python3
"""Same semantic German cases as desktop make-fixtures.ps1, macOS Anna voice."""
import argparse, pathlib, json, subprocess
p=argparse.ArgumentParser();p.add_argument('output',type=pathlib.Path);a=p.parse_args()
a.output.mkdir(parents=True,exist_ok=True)
cases=json.loads((pathlib.Path(__file__).resolve().parents[1]/'tests/german-cases.json').read_text())
for case in cases:
    aiff=a.output/(case['id']+'.aiff'); m4a=a.output/(case['id']+'.m4a')
    subprocess.run(['say','-v','Anna','-o',str(aiff),case['text']],check=True)
    subprocess.run(['afconvert','-f','m4af','-d','aac','-b','32000',str(aiff),str(m4a)],check=True)
    print(case['id'])

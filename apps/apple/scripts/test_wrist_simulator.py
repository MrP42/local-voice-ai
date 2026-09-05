#!/usr/bin/env python3
"""Run from repo root with the documented P0/P1 simulator pair open. Records up to 30 seconds using simulator microphone input."""
import subprocess,pathlib,json,sys
cmd=['xcodebuild','test','-project','apps/apple/LocalVoice.xcodeproj','-scheme','SimulatorControlsUITests','-destination','platform=macOS','-derivedDataPath','apps/apple/DerivedData','-only-testing:SimulatorControlsUITests/SimulatorControlsTests/testLowerWrist']
started=False
with open('/tmp/local-voice-wrist-down-final.log','w') as log:
 proc=subprocess.Popen(cmd,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,bufsize=1)
 for line in proc.stdout:
  log.write(line);log.flush()
  if 'WRIST_CONTROL_READY' in line:
   subprocess.run(['xcrun','simctl','launch','--terminate-running-process','41324A06-70C6-4C60-AE7F-B62EECB30F91','de.localvoice.prototype.watchkitapp','--record-probe'],check=True)
   started=True;print('Recorder launched at ready marker',flush=True)
 code=proc.wait()
assert started and code==0,('Wrist harness failed',started,code)
print('PASS: recording started and Wrist Down selected',flush=True)

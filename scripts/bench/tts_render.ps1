# Render utterances with Windows SAPI voices to 16 kHz mono 16-bit WAV (ASCII-only script).
# Usage: pwsh -NoProfile -File tts_render.ps1 -Jobs jobs.json
#   jobs.json: [{"voice":"Microsoft Stefan","text":"...","out":"C:\\...\\x.wav"}, ...]  (UTF-8)
# Needs PowerShell 7 (pwsh): only there the OneCore voices (Hedda/Katja/Stefan) are visible.
param([Parameter(Mandatory = $true)][string]$Jobs)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
$list = Get-Content -Raw -Encoding UTF8 $Jobs | ConvertFrom-Json
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$synth = @{}
foreach ($j in $list) {
  if (-not $synth.ContainsKey($j.voice)) {
    $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
    $s.SelectVoice($j.voice)
    $synth[$j.voice] = $s
  }
  $s = $synth[$j.voice]
  $s.SetOutputToWaveFile($j.out, $fmt)
  $s.Speak($j.text)
  $s.SetOutputToNull()
}
foreach ($s in $synth.Values) { $s.Dispose() }
Write-Output ("rendered {0}" -f @($list).Count)

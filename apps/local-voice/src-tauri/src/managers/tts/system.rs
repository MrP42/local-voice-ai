//! Built-in macOS speech; no downloaded model, Python or server.
//! Text is passed through stdin, never through a shell or process arguments.
use super::engine::{EngineCaps, SynthesisRequest, TtsEngine, TtsEngineKind};
use std::io::Write;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct SystemVoice {
    pub name: String,
    pub language: String,
}

pub fn parse_voices(output: &str) -> Vec<SystemVoice> {
    output
        .lines()
        .filter_map(|line| {
            let (head, _) = line.split_once('#')?;
            let head = head.trim();
            let split = head.rfind(char::is_whitespace)?;
            let name = head[..split].trim();
            let language = head[split..].trim();
            if name.is_empty() || language.is_empty() {
                return None;
            }
            Some(SystemVoice {
                name: name.into(),
                language: language.replace('_', "-"),
            })
        })
        .collect()
}

pub fn available() -> bool {
    cfg!(target_os = "macos") && std::path::Path::new("/usr/bin/say").is_file()
}

pub fn voices() -> Vec<SystemVoice> {
    if !available() {
        return vec![];
    }
    // This lists voices installed by macOS; it never downloads voices.
    std::process::Command::new("/usr/bin/say")
        .args(["-v", "?"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_voices(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
pub struct SystemSpeech;

impl TtsEngine for SystemSpeech {
    fn kind(&self) -> TtsEngineKind {
        TtsEngineKind::System
    }
    fn caps(&self) -> EngineCaps {
        EngineCaps {
            style_tags: false,
            cloning: false,
            voice_switching: false,
            streaming: false,
            needs_gpu: false,
            export_formats: &["wav"],
        }
    }
    fn cache_tag(&self, voice: Option<&str>) -> String {
        let default_voice = crate::apple_speech::system_default_voice();
        format!(
            "macos-system/{}",
            voice.or(default_voice.as_deref()).unwrap_or("@default")
        )
    }
    async fn ensure_ready(&self) -> Result<(), String> {
        if available() {
            Ok(())
        } else {
            Err("macOS system speech is unavailable".into())
        }
    }
    async fn synthesize(&self, req: SynthesisRequest<'_>) -> Result<Vec<u8>, String> {
        self.ensure_ready().await?;
        let text = req.text.to_owned();
        let voice = req.voice.map(str::to_owned);
        tauri::async_runtime::spawn_blocking(move || synthesize(&text, voice.as_deref()))
            .await
            .map_err(|e| e.to_string())?
    }
}

pub fn synthesize(text: &str, voice: Option<&str>) -> Result<Vec<u8>, String> {
    if !available() {
        return Err("macOS system speech is unavailable".into());
    }
    // Validate against the installed list instead of allowing say to choose a
    // different voice silently when a saved voice was removed in macOS.
    if let Some(name) = voice {
        if !voices().iter().any(|v| v.name == name) {
            return Err(format!("macOS voice is no longer installed: {name}"));
        }
    }
    let out = tempfile::Builder::new()
        .prefix("lva-system-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| e.to_string())?
        .into_temp_path();
    let mut cmd = std::process::Command::new("/usr/bin/say");
    cmd.args(["--file-format=WAVE", "--data-format=LEI16@22050", "-o"])
        .arg(&out);
    if let Some(voice) = voice {
        cmd.arg("-v").arg(voice);
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // The manager bounds every synthesis request to a sentence.
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(error) = stdin.write_all(text.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => return Err(format!("macOS speech failed: {status}")),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("macOS speech timed out".into());
            }
        }
    }
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    if !super::protocol::looks_like_wav(&bytes) {
        return Err("macOS speech returned invalid audio".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_installed_voices_with_spaces_and_unicode() {
        let voices = parse_voices(
            "Anna       de_DE # Hallo\nBad News   en_US # Hello\nAmélie  fr_CA # Bonjour\ninvalid",
        );
        assert_eq!(voices.len(), 3);
        assert_eq!(voices[1].name, "Bad News");
        assert_eq!(voices[2].name, "Amélie");
        assert_eq!(voices[0].language, "de-DE");
    }
    #[test]
    fn system_cache_is_distinct_from_fish_and_piper() {
        let e = SystemSpeech;
        assert_ne!(
            e.cache_tag(None),
            super::super::engine::fish_cache_tag(None)
        );
        assert_ne!(e.cache_tag(Some("Samantha")), e.cache_tag(Some("Anna")));
        assert!(!e.caps().needs_gpu && !e.caps().cloning);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn installed_system_voice_synthesizes_without_a_module_download() {
        assert!(available());
        assert!(!voices().is_empty());
        let wav = synthesize("Guten Tag.", None).unwrap();
        assert!(super::super::protocol::looks_like_wav(&wav));
        assert!(wav.len() > 1000);
    }
}

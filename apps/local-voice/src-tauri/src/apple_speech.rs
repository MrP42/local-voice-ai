//! macOS Speech framework. Network recognition is never permitted.
use crate::managers::transcription::TimedSegment;
use std::path::Path;

#[derive(serde::Deserialize)]
pub struct SpeechResult {
    pub text: String,
    #[serde(default)]
    pub segments: Vec<TimedSegment>,
    #[serde(default)]
    error: String,
}

fn decode_result(json: &str) -> anyhow::Result<SpeechResult> {
    let result: SpeechResult = serde_json::from_str(json)?;
    if !result.error.is_empty() {
        anyhow::bail!("{}", result.error);
    }
    Ok(result)
}

pub const MODEL_ID: &str = "apple-speech";

pub fn model_info() -> crate::managers::model::ModelInfo {
    use crate::managers::model::{EngineType, ModelInfo, ModelSource};
    ModelInfo {
        id: MODEL_ID.into(),
        name: "Apple Speech".into(),
        description: "macOS · on-device speech recognition · no app download".into(),
        filename: String::new(),
        source: ModelSource::Local,
        size_mb: 0,
        is_downloaded: available("auto"),
        is_downloading: false,
        partial_size: 0,
        is_directory: false,
        engine_type: EngineType::AppleSpeech,
        accuracy_score: 0.0,
        speed_score: 0.0,
        supports_translation: false,
        is_recommended: true,
        supported_languages: installed_languages(),
        supports_language_selection: true,
        is_custom: false,
        supports_streaming: false,
        supports_language_detection: false,
        supports_stream_lookahead: false,
    }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn lva_speech_locales() -> *mut std::ffi::c_char;
    fn lva_speech_supported(language: *const std::ffi::c_char) -> std::ffi::c_int;
    fn lva_speech_authorized() -> std::ffi::c_int;
    fn lva_speech_transcribe(
        file: *const std::ffi::c_char,
        language: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_char;
    fn lva_system_default_voice() -> *mut std::ffi::c_char;
    fn lva_speech_free(value: *mut std::ffi::c_char);
}

pub fn installed_languages() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        take_string(unsafe { lva_speech_locales() })
            .unwrap_or_default()
            .lines()
            .map(|line| line.replace('_', "-"))
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        vec![]
    }
}

pub fn available(language: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Ok(language) = std::ffi::CString::new(language) else {
            return false;
        };
        unsafe { lva_speech_supported(language.as_ptr()) == 1 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = language;
        false
    }
}

pub fn authorized() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe { lva_speech_authorized() == 1 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

pub fn system_default_voice() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        take_string(unsafe { lva_system_default_voice() })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn take_string(value: *mut std::ffi::c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        lva_speech_free(value);
    }
    Some(text)
}

pub fn transcribe_wav(path: &Path, language: &str) -> anyhow::Result<SpeechResult> {
    #[cfg(target_os = "macos")]
    {
        let file = std::ffi::CString::new(path.to_string_lossy().as_bytes())?;
        let locale = std::ffi::CString::new(language)?;
        let result = take_string(unsafe { lva_speech_transcribe(file.as_ptr(), locale.as_ptr()) })
            .ok_or_else(|| anyhow::anyhow!("Apple speech returned no result"))?;
        decode_result(&result)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, language);
        anyhow::bail!("Apple speech is available on macOS only")
    }
}

pub fn transcribe(audio: &[f32], language: &str) -> anyhow::Result<String> {
    transcribe_audio(audio, language).map(|result| result.text)
}

pub fn transcribe_segments(audio: &[f32], language: &str) -> anyhow::Result<Vec<TimedSegment>> {
    transcribe_audio(audio, language).map(|result| result.segments)
}

fn transcribe_audio(audio: &[f32], language: &str) -> anyhow::Result<SpeechResult> {
    if !available(language) {
        anyhow::bail!("Apple offline speech recognition is unavailable for this language");
    }
    let path = tempfile::Builder::new()
        .prefix("lva-apple-speech-")
        .suffix(".wav")
        .tempfile()?
        .into_temp_path();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for sample in audio {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    transcribe_wav(&path, language)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speech_result_preserves_native_word_times() {
        let result = decode_result(r#"{"text":"Guten Tag.","segments":[{"text":"Guten","start_ms":170,"end_ms":520},{"text":"Tag.","start_ms":530,"end_ms":900}]}"#).unwrap();
        assert_eq!(result.text, "Guten Tag.");
        assert_eq!(result.segments[0].start_ms, 170);
        assert_eq!(result.segments[1].end_ms, 900);
    }
    #[test]
    fn permission_error_is_not_an_empty_successful_transcript() {
        assert!(decode_result(r#"{"text":"","error":"Permission denied"}"#).is_err());
    }
    #[test]
    fn builtin_needs_no_model_file_and_cannot_translate() {
        let model = model_info();
        assert!(model.filename.is_empty());
        assert_eq!(model.size_mb, 0);
        assert!(!model.supports_translation);
    }
}

// Re-export all audio components
mod device;
pub mod loopback;
pub mod loopback_timeline;
mod recorder;
mod resampler;
mod utils;
mod visualizer;
pub mod wav_writer;

pub use device::{list_input_devices, list_output_devices, CpalDeviceInfo};
pub use loopback::{downmix_to_mono, f32_to_i16, LoopbackCapture, QpcStamper};
pub use loopback_timeline::{LoopbackTimeline, TimelineAction};
pub use recorder::{
    is_microphone_access_denied, is_no_input_device_error, AudioRecorder, VadPolicy,
};
pub use resampler::FrameResampler;
pub use utils::{read_wav_samples, save_wav_file, verify_wav_file};
pub use visualizer::AudioVisualiser;
pub use wav_writer::StreamingWavWriter;

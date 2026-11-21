use super::*;
use crate::ndi::NDIlibAudioFrameV3;

type FreeAudioFn = unsafe extern "C" fn(*mut c_void, *const NDIlibAudioFrameV3);

pub(crate) struct AudioReceiver {
    receiver: Receiver,
    free: FreeAudioFn,
}

impl AudioReceiver {
    pub(crate) fn new(source: &NdiSource) -> Result<Self, String> {
        let receiver = Receiver::new(source, 10)?;
        let free = receiver._runtime.symbol(b"NDIlib_recv_free_audio_v3\0")?;
        Ok(Self { receiver, free })
    }

    pub(crate) fn poll_routed(
        &mut self,
        routing: &crate::audio_input_config::InputRoutingPlan,
    ) -> Result<Option<(u32, Vec<f32>)>, String> {
        self.poll_with(|audio| unsafe { copy_audio(audio, routing) })
    }

    fn poll_with<T>(
        &mut self,
        copy: impl FnOnce(&NDIlibAudioFrameV3) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let mut audio: NDIlibAudioFrameV3 = unsafe { std::mem::zeroed() };
        let kind = unsafe {
            (self.receiver.capture)(
                self.receiver.handle,
                std::ptr::null_mut(),
                &mut audio as *mut _ as *mut c_void,
                std::ptr::null_mut(),
                100,
            )
        };
        match kind {
            2 => {
                let result = copy(&audio);
                unsafe { (self.free)(self.receiver.handle, &audio) };
                result.map(Some)
            }
            -1 => Err("NDI audio receiver connection failed.".into()),
            _ => Ok(None),
        }
    }
}

unsafe fn copy_audio(
    frame: &NDIlibAudioFrameV3,
    routing: &crate::audio_input_config::InputRoutingPlan,
) -> Result<(u32, Vec<f32>), String> {
    validate_audio_frame(frame)?;
    if frame.no_channels < routing.source_channels as i32 {
        return Err("Selected NDI audio source channel is unavailable.".into());
    }
    let mut samples = Vec::with_capacity(frame.no_samples as usize * routing.bus_channels);
    let mut output = [0.0; crate::audio_input_config::MAX_AUDIO_INPUT_BUSES];
    for index in 0..frame.no_samples as usize {
        routing.map_channel_frame(
            |channel| {
                let offset = channel * frame.channel_stride_in_bytes as usize + index * 4;
                frame.p_data.add(offset).cast::<f32>().read_unaligned()
            },
            &mut output,
        );
        samples.extend_from_slice(&output[..routing.bus_channels]);
    }
    Ok((frame.sample_rate as u32, samples))
}

unsafe fn validate_audio_frame(frame: &NDIlibAudioFrameV3) -> Result<(), String> {
    if !(8_000..=192_000).contains(&frame.sample_rate)
        || !(1..=64).contains(&frame.no_channels)
        || frame.no_samples <= 0
        || frame.no_samples > frame.sample_rate
        || frame.four_cc != u32::from_le_bytes(*b"FLTp")
        || frame.p_data.is_null()
        || frame.channel_stride_in_bytes < frame.no_samples * 4
        || frame.channel_stride_in_bytes > 4 * 192_000
    {
        return Err("NDI returned an invalid or unsupported audio frame.".into());
    }
    Ok(())
}

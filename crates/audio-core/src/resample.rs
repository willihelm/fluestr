//! Sample-rate conversion to the 16 kHz mono f32 whisper.cpp expects (P2).
//!
//! Intercom interfaces run at 44.1 or 48 kHz — nothing runs at 16 kHz — so
//! every stream passes through here. This is deliberately the *processing*
//! thread's job, never the audio callback's: sinc interpolation allocates
//! nothing but does cost real CPU per block, and a callback that overruns its
//! deadline produces clicks on the operator's headphones.
//!
//! Why sinc and not "take every third sample": decimating without a low-pass
//! folds everything above 8 kHz back into the audible band. Sibilants and
//! intercom hiss would land on top of the speech as alias tones, which is
//! exactly the material whisper is worst at. [`Resampler`] is verified against
//! that failure in its own tests.

use crate::ring::RingSource;
use crate::{AudioError, Result, SAMPLE_RATE};
use rubato::{
    calculate_cutoff, Resampler as _, SincFixedIn, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

/// Input frames handed to rubato per call — about 21 ms at 48 kHz.
///
/// The chunk is a latency-versus-overhead trade-off: rubato only produces
/// output once a full chunk has arrived, so this size is a floor under the
/// conversion latency. 1024 keeps that floor an order of magnitude below the
/// segment lengths P3 works with while still amortising the per-call overhead.
const CHUNK_FRAMES: usize = 1024;

/// Length of the windowed sinc filter.
///
/// 128 rather than rubato's suggested 256: the filter length is the other half
/// of the latency floor (half of it, in input samples, is pure group delay),
/// and 128 taps with a Blackman-Harris² window already put the stopband far
/// below what a 16 kHz speech model can resolve. "Latency first" is the
/// project's stated priority.
const SINC_LEN: usize = 128;

/// Intermediate points per input sample. Costs memory for the filter table,
/// not CPU per sample.
const OVERSAMPLING: usize = 128;

fn window() -> WindowFunction {
    WindowFunction::BlackmanHarris2
}

/// Converts a mono f32 stream from the device rate to [`SAMPLE_RATE`].
///
/// Stateful and per stream: the filter carries history across calls, so feeding
/// two streams through one instance would smear them into each other.
pub struct Resampler {
    input_rate: u32,
    /// `None` when the device already runs at 16 kHz and the samples only need
    /// range-clamping. Rare in practice, but a resampler at ratio 1.0 would add
    /// latency and CPU for nothing.
    inner: Option<Inner>,
}

struct Inner {
    resampler: SincFixedIn<f32>,
    /// Input samples that have not yet completed a chunk.
    pending: Vec<f32>,
    /// Pre-allocated non-interleaved buffers, one channel each.
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
}

impl Resampler {
    pub fn new(input_rate: u32) -> Result<Self> {
        if input_rate == 0 {
            return Err(AudioError::Resample("input sample rate is zero".into()));
        }

        if input_rate == SAMPLE_RATE {
            return Ok(Self {
                input_rate,
                inner: None,
            });
        }

        let parameters = SincInterpolationParameters {
            sinc_len: SINC_LEN,
            // Derived from the filter length rather than picked by hand: too
            // high a cutoff for a short filter is precisely how aliasing gets
            // back in.
            f_cutoff: calculate_cutoff(SINC_LEN, window()),
            oversampling_factor: OVERSAMPLING,
            // Linear between the oversampled points. Cubic buys accuracy that
            // is inaudible here and costs time in the per-block path.
            interpolation: SincInterpolationType::Linear,
            window: window(),
        };

        let ratio = SAMPLE_RATE as f64 / input_rate as f64;
        // `max_resample_ratio_relative` of 1.0: the hardware rate is fixed for
        // the life of a session, so no adjustment is ever asked for.
        let resampler = SincFixedIn::<f32>::new(ratio, 1.0, parameters, CHUNK_FRAMES, 1)
            .map_err(|e| AudioError::Resample(e.to_string()))?;

        let input = resampler.input_buffer_allocate(true);
        let output = resampler.output_buffer_allocate(true);

        Ok(Self {
            input_rate,
            inner: Some(Inner {
                resampler,
                pending: Vec::with_capacity(2 * CHUNK_FRAMES),
                input,
                output,
            }),
        })
    }

    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    /// True when the device already delivers [`SAMPLE_RATE`] and samples pass
    /// through unconverted.
    pub fn is_pass_through(&self) -> bool {
        self.inner.is_none()
    }

    /// Converts `input` and **appends** the result to `out`.
    ///
    /// Appending rather than returning a `Vec` keeps the caller's buffer alive
    /// across calls, so the steady state allocates nothing. Whatever does not
    /// complete a chunk stays buffered until the next call — a call may
    /// therefore legitimately produce no output at all.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<()> {
        let Some(inner) = self.inner.as_mut() else {
            out.extend(input.iter().map(|s| s.clamp(-1.0, 1.0)));
            return Ok(());
        };

        inner.pending.extend_from_slice(input);

        loop {
            let chunk = inner.resampler.input_frames_next();
            if inner.pending.len() < chunk {
                break;
            }

            inner.input[0][..chunk].copy_from_slice(&inner.pending[..chunk]);
            let (_, written) = inner
                .resampler
                .process_into_buffer(&inner.input, &mut inner.output, None)
                .map_err(|e| AudioError::Resample(e.to_string()))?;

            // Sinc interpolation overshoots around transients, so a signal that
            // peaked at full scale can leave the filter slightly above it.
            // whisper.cpp documents f32 input in [-1, 1].
            out.extend(
                inner.output[0][..written]
                    .iter()
                    .map(|s| s.clamp(-1.0, 1.0)),
            );
            inner.pending.drain(..chunk);
        }

        Ok(())
    }

    /// Pushes the buffered remainder out, zero-padded to a full chunk.
    ///
    /// For the end of a stream only. The filter's own group delay is not
    /// chased any further — it is well under a hundred input samples, and
    /// trailing silence is not worth a second padded pass.
    pub fn flush(&mut self, out: &mut Vec<f32>) -> Result<()> {
        let Some(inner) = self.inner.as_mut() else {
            return Ok(());
        };
        if inner.pending.is_empty() {
            return Ok(());
        }

        let chunk = inner.resampler.input_frames_next();
        let taken = inner.pending.len().min(chunk);
        inner.input[0][..taken].copy_from_slice(&inner.pending[..taken]);
        inner.input[0][taken..chunk].fill(0.0);

        let (_, written) = inner
            .resampler
            .process_into_buffer(&inner.input, &mut inner.output, None)
            .map_err(|e| AudioError::Resample(e.to_string()))?;

        out.extend(
            inner.output[0][..written]
                .iter()
                .map(|s| s.clamp(-1.0, 1.0)),
        );
        inner.pending.drain(..taken);
        Ok(())
    }

    /// Samples still held back because they do not complete a chunk.
    pub fn buffered(&self) -> usize {
        self.inner.as_ref().map_or(0, |inner| inner.pending.len())
    }
}

/// One stream's processing side: drain the ring, resample, hand on 16 kHz mono.
///
/// Deliberately not a thread. The caller owns the scheduling — P3 will pull
/// from several of these on one processing thread, and a thread per stream
/// would put eight of them in contention over the same cores the workers need.
pub struct StreamProcessor {
    source: RingSource,
    resampler: Resampler,
    /// Scratch for the ring read, sized so one call drains a typical backlog.
    scratch: Vec<f32>,
}

impl StreamProcessor {
    pub fn new(source: RingSource, input_rate: u32) -> Result<Self> {
        Ok(Self {
            source,
            resampler: Resampler::new(input_rate)?,
            scratch: vec![0.0; 4 * CHUNK_FRAMES],
        })
    }

    /// Moves everything currently in the ring through the resampler, appending
    /// 16 kHz mono samples to `out`. Returns how many it appended.
    pub fn pump(&mut self, out: &mut Vec<f32>) -> Result<usize> {
        let before = out.len();
        loop {
            let read = self.source.read(&mut self.scratch);
            if read == 0 {
                break;
            }
            self.resampler.process(&self.scratch[..read], out)?;
        }
        Ok(out.len() - before)
    }

    pub fn source(&self) -> &RingSource {
        &self.source
    }

    pub fn resampler(&self) -> &Resampler {
        &self.resampler
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ring, SampleSink};
    use std::f32::consts::TAU;

    /// Magnitude of `signal` at `freq`, by Goertzel.
    ///
    /// A full FFT would need a dependency; one bin at a chosen frequency is all
    /// these assertions ask for, and writing it out keeps the test honest about
    /// what it measures.
    fn magnitude_at(signal: &[f32], freq: f32, rate: u32) -> f32 {
        let omega = TAU * freq / rate as f32;
        let (cos, sin) = (omega.cos(), omega.sin());
        let coeff = 2.0 * cos;

        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &sample in signal {
            let s0 = sample + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }

        let real = s1 - s2 * cos;
        let imag = s2 * sin;
        // Normalised so a unit-amplitude sine reads back as roughly 1.0.
        2.0 * (real * real + imag * imag).sqrt() / signal.len() as f32
    }

    fn sine(freq: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let samples = (rate as f32 * seconds) as usize;
        (0..samples)
            .map(|n| (TAU * freq * n as f32 / rate as f32).sin())
            .collect()
    }

    /// Resamples in realistic block sizes rather than one giant call, because
    /// that is how the processing thread will drive it.
    fn resample_all(input: &[f32], rate: u32, block: usize) -> Vec<f32> {
        let mut resampler = Resampler::new(rate).unwrap();
        let mut out = Vec::new();
        for chunk in input.chunks(block) {
            resampler.process(chunk, &mut out).unwrap();
        }
        resampler.flush(&mut out).unwrap();
        out
    }

    /// Skips the filter's start-up transient before measuring.
    fn steady_state(signal: &[f32]) -> &[f32] {
        let skip = SINC_LEN.min(signal.len() / 4);
        &signal[skip..]
    }

    #[test]
    fn a_tone_keeps_its_frequency_and_amplitude() {
        // 1 kHz is squarely inside the speech band and well below both Nyquist
        // limits, so nothing but filter error can move it.
        let input = sine(1_000.0, 48_000, 1.0);
        let out = resample_all(&input, 48_000, 480);
        let out = steady_state(&out);

        let at_1k = magnitude_at(out, 1_000.0, SAMPLE_RATE);
        assert!(
            (at_1k - 1.0).abs() < 0.05,
            "1 kHz should survive at full amplitude, measured {at_1k}"
        );

        // Nothing else should have appeared. A frequency shift would show up
        // as energy next to the original bin.
        for probe in [500.0, 2_000.0, 3_000.0, 6_000.0] {
            let leaked = magnitude_at(out, probe, SAMPLE_RATE);
            assert!(leaked < 0.02, "spurious energy at {probe} Hz: {leaked}");
        }
    }

    #[test]
    fn content_above_the_new_nyquist_is_rejected_not_folded_back() {
        // The decisive test for P2. 12 kHz at 48 kHz input is above the 8 kHz
        // Nyquist of the 16 kHz output. Naive decimation would mirror it to
        // 4 kHz — right in the middle of the speech band and indistinguishable
        // from real signal afterwards.
        let input = sine(12_000.0, 48_000, 1.0);
        let out = resample_all(&input, 48_000, 480);
        let out = steady_state(&out);

        let alias = magnitude_at(out, 4_000.0, SAMPLE_RATE);
        assert!(
            alias < 0.01,
            "12 kHz must not alias to 4 kHz, measured {alias}"
        );

        let total: f32 = out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32;
        assert!(
            total.sqrt() < 0.05,
            "the tone should be suppressed entirely, rms {}",
            total.sqrt()
        );
    }

    #[test]
    fn the_odd_ratio_of_44100_hz_works_too() {
        // 44.1 -> 16 kHz is not an integer ratio; it is also the default rate
        // of the SoundGrid interface this app targets first.
        let input = sine(2_000.0, 44_100, 1.0);
        let out = resample_all(&input, 44_100, 441);
        let out = steady_state(&out);

        let at_2k = magnitude_at(out, 2_000.0, SAMPLE_RATE);
        assert!((at_2k - 1.0).abs() < 0.05, "2 kHz measured {at_2k}");
    }

    #[test]
    fn output_length_follows_the_rate_ratio() {
        let input = sine(1_000.0, 48_000, 1.0);
        let out = resample_all(&input, 48_000, 1024);

        // 48000 in at ratio 1/3 = 16000 out. The flush pads the last partial
        // chunk with zeros, so the tail can run slightly long — trailing
        // silence, which costs nothing downstream.
        assert!(
            (15_800..=16_200).contains(&out.len()),
            "expected about 16000 samples, got {}",
            out.len()
        );
    }

    #[test]
    fn a_16_khz_device_passes_through_untouched() {
        let mut resampler = Resampler::new(SAMPLE_RATE).unwrap();
        assert!(resampler.is_pass_through());

        let input = vec![0.1, -0.2, 0.3];
        let mut out = Vec::new();
        resampler.process(&input, &mut out).unwrap();

        assert_eq!(out, input);
        assert_eq!(resampler.buffered(), 0);
    }

    #[test]
    fn output_stays_inside_the_range_whisper_expects() {
        // Full-scale square-ish content is where sinc overshoot appears.
        let input: Vec<f32> = (0..48_000)
            .map(|n| if (n / 24) % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let out = resample_all(&input, 48_000, 512);

        assert!(!out.is_empty());
        assert!(
            out.iter().all(|s| (-1.0..=1.0).contains(s)),
            "samples must stay in [-1, 1]"
        );
    }

    #[test]
    fn partial_chunks_are_buffered_rather_than_dropped() {
        let mut resampler = Resampler::new(48_000).unwrap();
        let mut out = Vec::new();

        // Less than one chunk: nothing can come out yet, but nothing is lost.
        resampler.process(&vec![0.5; 100], &mut out).unwrap();
        assert!(out.is_empty());
        assert_eq!(resampler.buffered(), 100);

        resampler
            .process(&vec![0.5; CHUNK_FRAMES], &mut out)
            .unwrap();
        assert!(!out.is_empty());
        assert!(resampler.buffered() < CHUNK_FRAMES);
    }

    #[test]
    fn block_size_does_not_change_the_result() {
        // The processing thread cannot control how much the ring hands it, so
        // the output has to be independent of that.
        let input = sine(1_500.0, 48_000, 0.5);
        let a = resample_all(&input, 48_000, 97);
        let b = resample_all(&input, 48_000, 4096);

        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-6, "{x} vs {y}");
        }
    }

    #[test]
    fn a_zero_sample_rate_is_rejected() {
        assert!(matches!(Resampler::new(0), Err(AudioError::Resample(_))));
    }

    #[test]
    fn the_processor_drains_the_ring_into_16_khz_samples() {
        let (mut sink, source) = ring::channel(48_000, 2.0);
        let mut processor = StreamProcessor::new(source, 48_000).unwrap();

        let input = sine(1_000.0, 48_000, 1.0);
        // Push in device-callback-sized blocks, pumping in between, the way the
        // two threads will actually interleave.
        let mut out = Vec::new();
        for block in input.chunks(512) {
            sink.push(block);
            processor.pump(&mut out).unwrap();
        }

        assert!(
            (15_000..=16_000).contains(&out.len()),
            "expected about a second at 16 kHz, got {}",
            out.len()
        );
        let at_1k = magnitude_at(steady_state(&out), 1_000.0, SAMPLE_RATE);
        assert!((at_1k - 1.0).abs() < 0.05, "1 kHz measured {at_1k}");
        assert_eq!(processor.source().stats().samples_dropped(), 0);
    }

    #[test]
    fn pumping_an_empty_ring_is_a_no_op() {
        let (_sink, source) = ring::channel(48_000, 1.0);
        let mut processor = StreamProcessor::new(source, 48_000).unwrap();

        let mut out = Vec::new();
        assert_eq!(processor.pump(&mut out).unwrap(), 0);
        assert!(out.is_empty());
    }

    /// Reports the realtime factor of the conversion. Ignored because the
    /// number is a property of the machine, not of the code — it feeds
    /// `docs/measurements.md` and the latency budget in P14.
    ///
    /// `cargo test -p audio-core --release -- --ignored --nocapture resampling_cost`
    #[test]
    #[ignore = "measurement, machine-dependent"]
    fn resampling_cost() {
        const SECONDS: f32 = 60.0;
        let input = sine(1_000.0, 48_000, SECONDS);
        let mut resampler = Resampler::new(48_000).unwrap();
        let mut out = Vec::with_capacity(SAMPLE_RATE as usize * SECONDS as usize);

        let start = std::time::Instant::now();
        for block in input.chunks(512) {
            resampler.process(block, &mut out).unwrap();
        }
        let elapsed = start.elapsed();

        println!(
            "48 kHz -> 16 kHz, one stream: {SECONDS} s of audio in {:.1} ms ({:.0}x realtime)",
            elapsed.as_secs_f64() * 1000.0,
            SECONDS as f64 / elapsed.as_secs_f64()
        );

        // Alias rejection, in the same run: a 12 kHz tone would land on 4 kHz
        // without the low-pass.
        let folded = resample_all(&sine(12_000.0, 48_000, 1.0), 48_000, 480);
        let alias = magnitude_at(steady_state(&folded), 4_000.0, SAMPLE_RATE);
        println!(
            "12 kHz tone folded to 4 kHz: {alias:.6} ({:.0} dB below full scale)",
            20.0 * alias.max(f32::MIN_POSITIVE).log10()
        );
    }
}

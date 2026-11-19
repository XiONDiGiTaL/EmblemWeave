//! The launcher's own frame timing: the rate in the top bar and the frame-time
//! graph with its 1 % and 0.1 % lows.
//!
//! This measures the launcher's window, not the game's: the game is a separate
//! process with its own swapchain, and its frame rate is the engine's to report.

use std::collections::VecDeque;
use std::time::Instant;

/// The last fifteen seconds or so at 60 frames a second.
pub struct Frames {
    times: VecDeque<f32>,
    last: Option<Instant>,
    /// Frames ignored after the window opens: the first ones carry the
    /// window's own start-up, not steady drawing.
    warm: u8,
}

impl Default for Frames {
    fn default() -> Self {
        Frames { times: VecDeque::with_capacity(Self::CAP), last: None, warm: 0 }
    }
}

impl Frames {
    pub const CAP: usize = 900;

    /// Record the time since the previous frame. Gaps longer than a second (a
    /// window drag, a sleep) are not frames and are dropped.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if self.warm < 30 {
            self.warm += 1;
            self.last = Some(now);
            return;
        }
        if let Some(prev) = self.last.replace(now) {
            let dt = now.duration_since(prev).as_secs_f32();
            if dt > 0.0 && dt < 1.0 {
                if self.times.len() >= Self::CAP {
                    self.times.pop_front();
                }
                self.times.push_back(dt);
            }
        }
    }

    /// The rate over the last half second, or `None` before two frames.
    pub fn fps(&self) -> Option<f32> {
        let n = self.times.len().min(30);
        if n == 0 {
            return None;
        }
        let sum: f32 = self.times.iter().rev().take(n).sum();
        (sum > 0.0).then(|| n as f32 / sum)
    }

    pub fn samples(&self) -> Vec<f32> {
        self.times.iter().copied().collect()
    }

    /// The sorted samples once, then every number below read from that order.
    /// The percentiles and both lows used to take a copy each and sort it, so
    /// one call sorted the same 900 samples three times — on every frame the
    /// Diagnostics page and the overview graph asked for them.
    pub fn stats(&self) -> Stats {
        let mut times: Vec<f32> = self.times.iter().copied().collect();
        let n = times.len();
        if n == 0 {
            return Stats::default();
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let sum: f32 = times.iter().sum();
        // A frame time at rank `p` of the sorted ones, in milliseconds.
        let pct = |p: f32| {
            let i = ((n as f32 * p).ceil() as usize).saturating_sub(1).min(n - 1);
            times[i] * 1000.0
        };
        // The rate of the slowest `1 - p` of the frames: the 1 % low is the worst
        // frame of the slowest hundredth, not whichever frame sits at that rank.
        let low = |p: f32| {
            let k = ((n as f32) * (1.0 - p)).ceil().max(1.0) as usize;
            1.0 / times[n - k.min(n)]
        };
        Stats {
            frames: n,
            avg_ms: sum / n as f32 * 1000.0,
            max_ms: times[n - 1] * 1000.0,
            p99_ms: pct(0.99),
            avg_fps: 1.0 / (sum / n as f32),
            low1_fps: low(0.99),
            low01_fps: low(0.999),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    pub frames: usize,
    pub avg_ms: f32,
    pub max_ms: f32,
    pub avg_fps: f32,
    /// The 99th percentile frame time, in milliseconds: what the graph scales
    /// to, so one rare hitch does not flatten everything else.
    pub p99_ms: f32,
    /// The rate at the 99th percentile frame time: the 1 % low.
    pub low1_fps: f32,
    /// The rate at the 99.9th percentile frame time: the 0.1 % low.
    pub low01_fps: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(times: &[f32]) -> Frames {
        let mut f = Frames::default();
        for t in times {
            if f.times.len() >= Frames::CAP {
                f.times.pop_front();
            }
            f.times.push_back(*t);
        }
        f
    }

    #[test]
    fn averages_and_lows_come_from_the_frame_times() {
        // 99 steady frames at 60 fps and one 100 ms hitch.
        let mut t = vec![1.0 / 60.0; 99];
        t.push(0.1);
        let s = with(&t).stats();
        assert_eq!(s.frames, 100);
        // 1.75 s of frames over 100 of them.
        assert!((s.avg_fps - 100.0 / 1.75).abs() < 0.05, "avg {}", s.avg_fps);
        // The single hitch is both the 1 % and the 0.1 % worst frame.
        assert!((s.low1_fps - 10.0).abs() < 0.01, "1% low {}", s.low1_fps);
        assert!((s.low01_fps - 10.0).abs() < 0.01, "0.1% low {}", s.low01_fps);
        assert!((s.max_ms - 100.0).abs() < 0.01);
        // The rank-based 99th percentile is still a normal frame.
        assert!((s.p99_ms - 1000.0 / 60.0).abs() < 0.5, "p99 {}", s.p99_ms);
    }

    #[test]
    fn a_clean_run_has_matching_lows() {
        let f = with(&[1.0 / 144.0; 200]);
        let s = f.stats();
        assert!((s.avg_fps - 144.0).abs() < 0.5);
        assert!((s.low1_fps - 144.0).abs() < 0.5);
        assert!((s.low01_fps - 144.0).abs() < 0.5);
    }

    #[test]
    fn no_frames_no_numbers() {
        let f = Frames::default();
        assert_eq!(f.fps(), None);
        assert_eq!(f.stats(), Stats::default());
    }
}

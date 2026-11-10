use std::sync::atomic::{AtomicU32, Ordering};

/// Lock-free `f32` cell (bit-cast through `AtomicU32`).
#[derive(Debug, Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }

    /// Raise the stored value to `v` if larger. Only valid for non-negative values,
    /// whose IEEE-754 bit patterns order the same way as the floats themselves.
    pub fn raise_to(&self, v: f32) {
        if v.is_finite() && v >= 0.0 {
            self.0.fetch_max(v.to_bits(), Ordering::Relaxed);
        }
    }

    /// Read and reset to zero (used for peak meters).
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

pub fn db_to_gain(db: f32) -> f32 {
    if db <= -96.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

pub fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

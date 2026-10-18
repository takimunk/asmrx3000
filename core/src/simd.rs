//! 4-lane f32 vector for the per-sample kernels: NEON intrinsics on aarch64 (Apple silicon,
//! ARM Linux/Windows), a plain [f32; 4] elsewhere (x86 and WASM builds rely on LLVM's
//! vectoriser for that path).

#[cfg(target_arch = "aarch64")]
mod imp {
    use core::arch::aarch64::*;

    #[derive(Clone, Copy)]
    pub struct F4(float32x4_t);

    impl F4 {
        #[inline(always)]
        pub fn zero() -> F4 {
            unsafe { F4(vdupq_n_f32(0.0)) }
        }
        #[inline(always)]
        pub fn splat(v: f32) -> F4 {
            unsafe { F4(vdupq_n_f32(v)) }
        }
        #[inline(always)]
        pub fn load(s: &[f32]) -> F4 {
            assert!(s.len() >= 4);
            unsafe { F4(vld1q_f32(s.as_ptr())) }
        }
        #[inline(always)]
        pub fn sum(self) -> f32 {
            unsafe { vaddvq_f32(self.0) }
        }
        /// self + a * b (fused).
        #[inline(always)]
        pub fn fma(self, a: F4, b: F4) -> F4 {
            unsafe { F4(vfmaq_f32(self.0, a.0, b.0)) }
        }
        /// self - a * b (fused).
        #[inline(always)]
        pub fn fms(self, a: F4, b: F4) -> F4 {
            unsafe { F4(vfmsq_f32(self.0, a.0, b.0)) }
        }
        #[inline(always)]
        pub fn mul(self, b: F4) -> F4 {
            unsafe { F4(vmulq_f32(self.0, b.0)) }
        }
        #[inline(always)]
        pub fn add(self, b: F4) -> F4 {
            unsafe { F4(vaddq_f32(self.0, b.0)) }
        }
        #[inline(always)]
        pub fn store(self, out: &mut [f32]) {
            assert!(out.len() >= 4);
            unsafe { vst1q_f32(out.as_mut_ptr(), self.0) }
        }
    }
}

#[cfg(not(target_arch = "aarch64"))]
mod imp {
    #[derive(Clone, Copy)]
    pub struct F4([f32; 4]);

    impl F4 {
        #[inline(always)]
        pub fn zero() -> F4 {
            F4([0.0; 4])
        }
        #[inline(always)]
        pub fn splat(v: f32) -> F4 {
            F4([v; 4])
        }
        #[inline(always)]
        pub fn load(s: &[f32]) -> F4 {
            F4([s[0], s[1], s[2], s[3]])
        }
        #[inline(always)]
        pub fn sum(self) -> f32 {
            (self.0[0] + self.0[1]) + (self.0[2] + self.0[3])
        }
        #[inline(always)]
        pub fn fma(self, a: F4, b: F4) -> F4 {
            F4(core::array::from_fn(|i| self.0[i] + a.0[i] * b.0[i]))
        }
        #[inline(always)]
        pub fn fms(self, a: F4, b: F4) -> F4 {
            F4(core::array::from_fn(|i| self.0[i] - a.0[i] * b.0[i]))
        }
        #[inline(always)]
        pub fn mul(self, b: F4) -> F4 {
            F4(core::array::from_fn(|i| self.0[i] * b.0[i]))
        }
        #[inline(always)]
        pub fn add(self, b: F4) -> F4 {
            F4(core::array::from_fn(|i| self.0[i] + b.0[i]))
        }
        #[inline(always)]
        pub fn store(self, out: &mut [f32]) {
            out[..4].copy_from_slice(&self.0);
        }
    }
}

pub use imp::F4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_ops() {
        let a = F4::load(&[1.0, 2.0, 3.0, 4.0]);
        let b = F4::splat(2.0);
        assert_eq!(a.mul(b).sum(), 20.0);
        assert_eq!(F4::zero().fma(a, b).add(a).sum(), 30.0);
        assert_eq!(a.fms(a, b).sum(), -10.0);
    }
}

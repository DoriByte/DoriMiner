
use prl_pow::{forma_legal, JobSide, Pair, Shape};
use std::ffi::{c_char, c_int, c_void, CStr};

pub const MAX_HITS: usize = 63;

extern "C" {
    fn prl_series(capability: u32, series: *mut u32, err: *mut c_char, errlen: c_int) -> c_int;
    fn prl_supported(capability: u32) -> c_int;
    fn prl_series_name(series: u32) -> *const c_char;
    fn prl_measured() -> c_int;
    fn prl_open(ordinal: c_int, series: u32, threads: u32, rows: RowsFn, err: *mut c_char, errlen: c_int) -> *mut c_void;
    fn prl_close(h: *mut c_void);
    fn prl_shape(h: *mut c_void, out: *mut u32);
    fn prl_info_a(h: *mut c_void) -> u64;
    fn prl_info_b(h: *mut c_void, row: u32, out: *mut u32) -> c_int;
    fn prl_job(h: *mut c_void, b: *const i8, pb: *const u32, pb_len: usize, seed_b: *const u8, err: *mut c_char, errlen: c_int) -> c_int;
    fn prl_prepare(h: *mut c_void, pm: *const u32, seed_a: *const u8, err: *mut c_char, errlen: c_int) -> *mut c_void;
    fn prl_prepare_drop(h: *mut c_void, tables: *mut c_void);
    fn prl_launch(h: *mut c_void, tables: *mut c_void, seed_a: *const u8, pos: u64, val: c_int, limit: *const u32, err: *mut c_char, errlen: c_int) -> *mut c_void;
    fn prl_read(h: *mut c_void, queued: *mut c_void, out: *mut u32, kept: *mut u32, err: *mut c_char, errlen: c_int) -> i64;
    fn prl_read_drop(h: *mut c_void, queued: *mut c_void);
}

type RowsFn = extern "C" fn(*const u8, *const u8, u32, u32, *mut u8);

extern "C" fn rows(seed: *const u8, label: *const u8, first: u32, count: u32, out: *mut u8) {
    unsafe {
        let out = std::slice::from_raw_parts_mut(out, count as usize * prl_pow::RANK);
        prl_pow::uniform_rows(&*(seed as *const [u8; 32]), &*(label as *const [u8; 8]), first, count, out);
    }
}

fn text(err: &[c_char]) -> String {
    unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Series(u32);

impl Series {
    pub fn for_card(capability: u32) -> Result<Self, String> {
        let (mut series, mut err) = (0u32, [0 as c_char; 256]);
        match unsafe { prl_series(capability, &mut series, err.as_mut_ptr(), err.len() as c_int) } {
            0 => Ok(Self(series)),
            _ => Err(text(&err)),
        }
    }

    pub fn name(self) -> String {
        unsafe { CStr::from_ptr(prl_series_name(self.0)) }.to_string_lossy().into_owned()
    }
}

pub fn supported(capability: u32) -> bool {
    unsafe { prl_supported(capability) != 0 }
}

pub fn measured() -> bool {
    unsafe { prl_measured() != 0 }
}

pub struct Tables {
    h: *mut c_void,
    p: *mut c_void,
}

impl Drop for Tables {
    fn drop(&mut self) {
        if !self.p.is_null() {
            unsafe { prl_prepare_drop(self.h, self.p) }
        }
    }
}

pub struct Queued {
    h: *mut c_void,
    p: *mut c_void,
}

impl Drop for Queued {
    fn drop(&mut self) {
        if !self.p.is_null() {
            unsafe { prl_read_drop(self.h, self.p) }
        }
    }
}

pub struct Searcher {
    h: *mut c_void,
    series: Series,
    shape: Shape,
}

impl Drop for Searcher {
    fn drop(&mut self) {
        unsafe { prl_close(self.h) }
    }
}

impl Searcher {
    pub fn new(ordinal: usize, series: Series) -> Result<Self, String> {
        let mut err = [0 as c_char; 256];
        let threads = prl_pow::threads() as u32;
        let h = unsafe { prl_open(ordinal as c_int, series.0, threads, rows, err.as_mut_ptr(), err.len() as c_int) };
        if h.is_null() {
            return Err(text(&err));
        }
        let mut s = [0u32; 5];
        unsafe { prl_shape(h, s.as_mut_ptr()) };
        let shape = Shape { m: s[0] as usize, n: s[1] as usize, k: s[2] as usize, na: s[3] as usize, h: s[4] };
        let me = Self { h, series, shape };
        forma_legal(&shape.config(), shape.m as u32, shape.n as u32, true).map_err(|e| format!("m {}: {e}", shape.m))?;
        Ok(me)
    }

    pub fn series(&self) -> Series {
        self.series
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    pub fn set_job(&mut self, job: &JobSide) -> Result<(), String> {
        assert_eq!(job.shape, self.shape(), "a job for another shape");
        let mut err = [0 as c_char; 256];
        let r = unsafe { prl_job(self.h, job.b_noisy.as_ptr(), job.pb.as_ptr(), job.pb.len(), job.seed_b.as_ptr(), err.as_mut_ptr(), err.len() as c_int) };
        if r == 0 {
            Ok(())
        } else {
            Err(text(&err))
        }
    }

    pub fn upload(&mut self, pair: &Pair) -> Result<Tables, String> {
        let mut err = [0 as c_char; 256];
        let p = unsafe { prl_prepare(self.h, pair.pm.as_ptr(), pair.seed_a.as_ptr(), err.as_mut_ptr(), err.len() as c_int) };
        if p.is_null() {
            return Err(text(&err));
        }
        Ok(Tables { h: self.h, p })
    }

    pub fn launch(&mut self, pair: &Pair, mut tables: Tables, limit: &[u32; 8]) -> Result<Queued, String> {
        let mut err = [0 as c_char; 256];
        let t = std::mem::replace(&mut tables.p, std::ptr::null_mut());
        let p = unsafe { prl_launch(self.h, t, pair.seed_a.as_ptr(), pair.pos as u64, pair.val as c_int, limit.as_ptr(), err.as_mut_ptr(), err.len() as c_int) };
        if p.is_null() {
            return Err(text(&err));
        }
        Ok(Queued { h: self.h, p })
    }

    pub fn range(&self) -> usize {
        unsafe { prl_info_a(self.h) as usize }
    }

    pub fn alt(&self, row: u32) -> Option<u32> {
        let mut out = 0u32;
        (unsafe { prl_info_b(self.h, row, &mut out) } != 0).then_some(out)
    }

    pub fn tiles(&self, queued: Queued) -> Result<Vec<(u32, u32)>, String> {
        Ok(self.tiles_counted(queued)?.0)
    }

    pub fn tiles_counted(&self, mut queued: Queued) -> Result<(Vec<(u32, u32)>, u32), String> {
        let (mut out, mut kept, mut err) = ([0u32; MAX_HITS], 0u32, [0 as c_char; 256]);
        let q = std::mem::replace(&mut queued.p, std::ptr::null_mut());
        let won = unsafe { prl_read(self.h, q, out.as_mut_ptr(), &mut kept, err.as_mut_ptr(), err.len() as c_int) };
        if won < 0 {
            return Err(text(&err));
        }
        Ok((out[..kept as usize].iter().map(|&w| (w >> 16, w & 0xffff)).collect(), won as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cards_with_an_image() {
        for capability in [120, 89, 86] {
            assert!(supported(capability));
        }
        assert!(!supported(75) && !supported(121) && !supported(90));
    }

    #[test]
    fn a_card_without_an_image_is_refused_in_words() {
        assert!(Series::for_card(75).unwrap_err().contains("sm_75"));
    }
}

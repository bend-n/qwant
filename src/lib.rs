/*
NeuQuant Neural-Net Quantization Algorithm by Anthony Dekker, 1994.
See "Kohonen neural networks for optimal colour quantization"
in "Network: Computation in Neural Systems" Vol. 5 (1994) pp 351-367.
for a discussion of the algorithm.
See also http://members.ozemail.com.au/~dekker/NEUQUANT.HTML

Incorporated bugfixes and alpha channel handling from pngnq
http://pngnq.sourceforge.net

Copyright (c) 2014 The Piston Developers

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.

NeuQuant Neural-Net Quantization Algorithm
------------------------------------------

Copyright (c) 1994 Anthony Dekker

NEUQUANT Neural-Net quantization algorithm by Anthony Dekker, 1994.
See "Kohonen neural networks for optimal colour quantization"
in "Network: Computation in Neural Systems" Vol. 5 (1994) pp 351-367.
for a discussion of the algorithm.
See also  http://members.ozemail.com.au/~dekker/NEUQUANT.HTML

Any party obtaining a copy of these files from the author, directly or
indirectly, is granted, free of charge, a full and unrestricted irrevocable,
world-wide, paid up, royalty-free, nonexclusive right and license to deal
in this software and documentation files (the "Software"), including without
limitation the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons who receive
copies from any such party to do so, with the only requirement being
that this copyright notice remain intact.

*/

//! # Color quantization library
//!
//! This library provides a color quantizer based on the [NEUQUANT](http://members.ozemail.com.au/~dekker/NEUQUANT.HTML)
//!
//! Original literature: Dekker, A. H. (1994). Kohonen neural networks for
//! optimal colour quantization. *Network: Computation in Neural Systems*, 5(3), 351-367.
//! [doi: 10.1088/0954-898X_5_3_003](https://doi.org/10.1088/0954-898X_5_3_003)
//!
//! See also <https://scientificgems.wordpress.com/stuff/neuquant-fast-high-quality-image-quantization/>
//!
//! ## Usage
//!
//! ```
//! # #![feature(generic_const_exprs)]
//! let data = vec![[0; 4]; 10];
//! let nq = qwant::NeuQuant::new(10, 256, &data);
//! let indixes: Vec<u8> = data.iter().map(|&pix| nq.index_of(pix) as u8).collect();
//! let color_map: Vec<[u8; 4]> = nq.take_color_map();
//! ```
#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
use atools::prelude::*;
use std::cmp::{max, min};
const RADIUS_DEC: i32 = 30; // factor of 1/30 each cycle

const ALPHA_BIASSHIFT: i32 = 10; // alpha starts at 1
const INIT_ALPHA: i32 = 1 << ALPHA_BIASSHIFT; // biased by 10 bits

const GAMMA: f64 = 1024.0;
const BETA: f64 = 1.0 / GAMMA;
const BETAGAMMA: f64 = BETA * GAMMA;

// four primes near 500 - assume no image has a length so large
// that it is divisible by all four primes
const PRIMES: [usize; 4] = [499, 491, 487, 503];

pub enum ControlFlow {
    Break,
    Continue,
}

impl ControlFlow {
    fn is_break(self) -> bool {
        matches!(self, ControlFlow::Break)
    }
}

type Neuron = [f64; 4];
pub type Color = [u8; 4];

pub struct NeuQuant {
    network: Vec<Neuron>,
    colormap: Vec<Color>,
    netindex: Box<[usize; 256]>,
    bias: Vec<f64>, // bias and freq arrays for learning
    freq: Vec<f64>,
    samplefac: i32,
    netsize: usize,
}

impl NeuQuant {
    /// Creates a new neuronal network and trains it with the supplied data.
    ///
    /// Pixels are assumed to be in RGBA format.
    /// `colors` should be $>=64$. `samplefac` determines the faction of
    /// the sample that will be used to train the network. Its value must be in the
    /// range $[1, 30]$. A value of $1$ thus produces the best result but is also
    /// slowest. $10$ is a good compromise between speed and quality.
    pub fn new(samplefac: i32, colors: usize, pixels: &[[u8; 4]]) -> Self {
        let netsize = colors;
        let mut this = NeuQuant {
            network: Vec::with_capacity(netsize),
            colormap: Vec::with_capacity(netsize),
            netindex: vec![0; 256].try_into().unwrap(),
            bias: Vec::with_capacity(netsize),
            freq: Vec::with_capacity(netsize),
            netsize: colors,
            samplefac,
        };
        this.init(pixels);
        this
    }

    /// Initializes the neuronal network and trains it with the supplied data.
    ///
    /// This method gets called by `Self::new`.
    pub fn init(&mut self, pixels: &[[u8; 4]]) {
        self.network.clear();
        self.colormap.clear();
        self.bias.clear();
        self.freq.clear();
        let freq = (self.netsize as f64).recip();
        for i in 0..self.netsize {
            let tmp = (i as f64) * 256.0 / (self.netsize as f64);
            // Sets alpha values at 0 for dark pixels.
            let a = if i < 16 { i as f64 * 16.0 } else { 255.0 };
            self.network.push([tmp, tmp, tmp, a]);
            self.colormap.push([0, 0, 0, 255]);
            self.freq.push(freq);
            self.bias.push(0.0);
        }
        self.learn(pixels);
        self.build_colormap();
        self.build_netindex();
    }

    /// Maps the rgba-pixel in-place to the best-matching color in the color map.
    #[inline(always)]
    pub fn map_pixel(&self, pixel: &mut [u8; 4]) {
        let [r, g, b, a] = *pixel;
        let i = self.search_netindex(b, g, r, a);
        *pixel = self.colormap[i];
    }

    /// Finds the best-matching index in the color map.
    #[inline(always)]
    pub fn index_of(&self, [r, g, b, a]: [u8; 4]) -> usize {
        self.search_netindex(b, g, r, a)
    }

    /// Lookup pixel values for color at `idx` in the colormap.
    pub fn lookup(&self, idx: usize) -> Option<[u8; 4]> {
        self.colormap.get(idx).copied()
    }

    /// Returns the color map.
    pub fn take_color_map(self) -> Vec<[u8; 4]> {
        self.colormap
    }

    /// Returns the RGBA color map calculated from the sample.
    pub fn color_map_rgba(&self) -> impl Iterator<Item = [u8; 4]> + '_ {
        self.colormap.iter().copied()
    }

    /// Returns the RGB color map calculated from the sample.
    pub fn color_map_rgb(&self) -> impl Iterator<Item = [u8; 3]> + '_ {
        self.colormap.iter().map(|q| q.trunc())
    }

    /// Move neuron i towards biased (a,b,g,r) by factor alpha
    fn salter_single(&mut self, alpha: f64, i: i32, quad: [f64; 4]) {
        let n = &mut self.network[i as usize];
        *n = n.asub(alpha.mul(n.asub(quad)));
    }

    /// Move neuron adjacent neurons towards biased (a,b,g,r) by factor alpha
    fn alter_neighbour(&mut self, alpha: f64, rad: i32, i: i32, quad: [f64; 4]) {
        let lo = max(i - rad, 0);
        let hi = min(i + rad, self.netsize as i32);
        let mut j = i + 1;
        let mut k = i - 1;
        let mut q = 0;

        while (j < hi) || (k > lo) {
            let rad_sq = rad as f64 * rad as f64;
            let alpha = (alpha * (rad_sq - q as f64 * q as f64)) / rad_sq;
            q += 1;
            if j < hi {
                let p = &mut self.network[j as usize];
                *p = p.asub(alpha.mul(p.asub(quad)));
                j += 1;
            }
            if k > lo {
                let p = &mut self.network[k as usize];
                *p = p.asub(alpha.mul(p.asub(quad)));
                k -= 1;
            }
        }
    }

    /// Search for biased BGR values
    /// finds closest neuron (min dist) and updates freq
    /// finds best neuron (min dist-bias) and returns position
    /// for frequently chosen neurons, freq[i] is high and bias[i] is negative
    /// bias[i] = gamma*((1/self.netsize)-freq[i])
    fn contest(&mut self, b: f64, g: f64, r: f64, a: f64) -> i32 {
        use std::f64;

        let mut bestd = f64::MAX;
        let mut bestbiasd: f64 = bestd;
        let mut bestpos = -1;
        let mut bestbiaspos: i32 = bestpos;

        for i in 0..self.netsize {
            let bestbiasd_biased = bestbiasd + self.bias[i];
            let mut dist;
            let &[nr, ng, nb, na] = &self.network[i];
            dist = (nb - b).abs();
            dist += (nr - r).abs();
            if dist < bestd || dist < bestbiasd_biased {
                dist += (ng - g).abs();
                dist += (na - a).abs();
                if dist < bestd {
                    bestd = dist;
                    bestpos = i as i32;
                }
                let biasdist = dist - self.bias[i];
                if biasdist < bestbiasd {
                    bestbiasd = biasdist;
                    bestbiaspos = i as i32;
                }
            }
            self.freq[i] -= BETA * self.freq[i];
            self.bias[i] += BETAGAMMA * self.freq[i];
        }
        self.freq[bestpos as usize] += BETA;
        self.bias[bestpos as usize] -= BETAGAMMA;
        bestbiaspos
    }

    /// Main learning loop
    /// Note: the number of learning cycles is crucial and the parameters are not
    /// optimized for net sizes < 26 or > 256. 1064 colors seems to work fine
    fn learn(&mut self, pixels: &[[u8; 4]]) {
        let initrad: i32 = self.netsize as i32 / 8; // for 256 cols, radius starts at 32
        let radiusbiasshift: i32 = 6;
        let radiusbias: i32 = 1 << radiusbiasshift;
        let init_bias_radius: i32 = initrad * radiusbias;
        let mut bias_radius = init_bias_radius;
        let alphadec = 30 + ((self.samplefac - 1) / 3);
        let lengthcount = pixels.len();
        let samplepixels = lengthcount / self.samplefac as usize;
        // learning cycles
        let n_cycles = match self.netsize >> 1 {
            n if n <= 100 => 100,
            n => n,
        };
        let delta = match samplepixels / n_cycles {
            0 => 1,
            n => n,
        };
        let mut alpha = INIT_ALPHA;

        let mut rad = bias_radius >> radiusbiasshift;
        if rad <= 1 {
            rad = 0
        };

        let mut pos = 0;
        let step = *PRIMES
            .iter()
            .find(|&&prime| lengthcount % prime != 0)
            .unwrap_or(&PRIMES[3]);

        let mut i = 0;
        while i < samplepixels {
            let [r, g, b, a] = pixels[pos].map(|x| x as f64);

            let j = self.contest(b, g, r, a);

            let alpha_ = (1.0 * alpha as f64) / INIT_ALPHA as f64;
            self.salter_single(alpha_, j, [r, g, b, a]);
            if rad > 0 {
                self.alter_neighbour(alpha_, rad, j, [r, g, b, a])
            };

            pos += step;
            while pos >= lengthcount {
                pos -= lengthcount
            }

            i += 1;
            if i % delta == 0 {
                alpha -= alpha / alphadec;
                bias_radius -= bias_radius / RADIUS_DEC;
                rad = bias_radius >> radiusbiasshift;
                if rad <= 1 {
                    rad = 0
                };
            }
        }
    }

    /// initializes the color map
    fn build_colormap(&mut self) {
        for (m, n) in self.colormap.iter_mut().zip(self.network.iter()) {
            *m = n.map(|x| x.round() as u8);
        }
    }

    /// Insertion sort of network and building of netindex[0..255]
    fn build_netindex(&mut self) {
        let mut previouscol = 0;
        let mut startpos = 0;

        for i in 0..self.netsize {
            let mut p = self.colormap[i];
            let mut q;
            let mut smallpos = i;
            let mut smallval = p[1] as usize; // index on g
                                              // find smallest in i..netsize-1
            for j in (i + 1)..self.netsize {
                q = self.colormap[j];
                if (q[1] as usize) < smallval {
                    // index on g
                    smallpos = j;
                    smallval = q[1] as usize; // index on g
                }
            }
            q = self.colormap[smallpos];
            // swap p (i) and q (smallpos) entries
            if i != smallpos {
                ::std::mem::swap(&mut p, &mut q);
                self.colormap[i] = p;
                self.colormap[smallpos] = q;
            }
            // smallval entry is now in position i
            if smallval != previouscol {
                self.netindex[previouscol] = (startpos + i) >> 1;
                for j in (previouscol + 1)..smallval {
                    self.netindex[j] = i
                }
                previouscol = smallval;
                startpos = i;
            }
        }
        let max_netpos = self.netsize - 1;
        self.netindex[previouscol] = (startpos + max_netpos) >> 1;
        for j in (previouscol + 1)..256 {
            self.netindex[j] = max_netpos
        } // really 256
    }
    /// Search for best matching color
    fn search_netindex(&self, b: u8, g: u8, r: u8, a: u8) -> usize {
        let mut best_dist = std::i32::MAX;
        let first_guess = self.netindex[g as usize];
        let mut best_pos = first_guess;
        let mut i = best_pos;

        #[inline]
        fn sqr_dist(a: i32, b: u8) -> i32 {
            let dist = a - b as i32;
            dist * dist
        }

        {
            let mut cmp = |i| {
                let x: [u8; 4] = self.colormap[i];
                let [pr, pg, pb, pa] = x.map(|x| x as i32);
                let mut dist = sqr_dist(pg, g);
                if dist > best_dist {
                    // If the green is less than optimal, break.
                    // Seems to be arbitrary choice from the original implementation,
                    // but can't change this w/o invasive changes since
                    // we also sort by green.
                    return ControlFlow::Break;
                }
                // otherwise, continue searching through the colormap.
                dist += sqr_dist(pr, r);
                if dist >= best_dist {
                    return ControlFlow::Continue;
                }
                dist += sqr_dist(pb, b);
                if dist >= best_dist {
                    return ControlFlow::Continue;
                }
                dist += sqr_dist(pa, a);
                if dist >= best_dist {
                    return ControlFlow::Continue;
                }
                best_dist = dist;
                best_pos = i;
                ControlFlow::Continue
            };
            while i < self.netsize {
                i = if cmp(i).is_break() { break } else { i + 1 };
            }
            // this j < C is a cheat to avoid the bounds check, as when the loop reaches 0
            // it will wrap to usize::MAX. Assume that self.netsize < usize::MAX, otherwise
            // using a lot of memory. This also must be true since netsize < isize::MAX <
            // usize::MAX, otherwise it would fail while allocating a Vec since they can be at
            // most isize::MAX elements.
            let mut j = first_guess.wrapping_sub(1);
            while j < self.netsize {
                j = if cmp(j).is_break() {
                    break;
                } else {
                    j.wrapping_sub(1)
                };
            }
        }

        best_pos
    }
}

# Color quantization library
This library provides a color quantizer based on the [NEUQUANT](https://scientificgems.wordpress.com/stuff/neuquant-fast-high-quality-image-quantization/)
quantization algorithm by Anthony Dekker.

### Usage

```rust
let data = vec![[0; 4]; 10];
let nq = qwant::NeuQuant::new(10, 256, &data);
let indixes: Vec<u8> = data.iter().map(|&pix| nq.index_of(pix) as u8).collect();
let color_map: Vec<[u8; 4]> = nq.take_color_map();
```
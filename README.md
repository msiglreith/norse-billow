
<font size=6>billow</font>
<br>
<font size=2>N O R S E</font>

---

<p align="center">
    <a href="LICENSE-MIT">
      <img src="https://img.shields.io/badge/license-MIT-green.svg?style=flat-square" alt="License - MIT">
    </a>
    <a href="LICENSE-APACHE">
      <img src="https://img.shields.io/badge/license-APACHE2-green.svg?style=flat-square" alt="License - Apache2">
  </a>
</p>

`billow` is an utility library for suballocating memory blocks in cache-friendly way for SoA data structures.

```toml
[dependencies]
norse-billow = "0.1"
```

## Usage

```rust
const NUM_ELEMENTS: usize = 128;

type Transform = [[f32; 4]; 4];
type Velocity = [f32; 3];

// Build layout for SoA:
//
// struct Block {
//     transforms: &mut [Transform],
//     velocity: &mut [Velocity],
// }
let mut layout = billow::BlockLayout::build();
let transform_id = layout.add::<Transform>();
let velocity_id = layout.add::<Velocity>();
let block_layout = layout.finish();

// Allocate memory block for holding `NUM_ELEMENTS` elements.
let layout = block_layout.layout();
let size = layout.size() * NUM_ELEMENTS;
let memory = unsafe {
    alloc::alloc(Layout::from_size_align(
        size, layout.align()
    )?)
};

let block = block_layout.apply(NonNull::new(memory).unwrap(), layout.size() * 128);
assert_eq!(block.len(), NUM_ELEMENTS);

let transforms = unsafe { block.as_slice::<Transform>(transform_id) };
let velocities = unsafe { block.as_slice::<Velocity>(velocity_id) };

assert_eq!(transforms.len(), velocities.len());
```

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any Contribution intentionally submitted for inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

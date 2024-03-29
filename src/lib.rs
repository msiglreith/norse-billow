/*!
Allocator for SoA data layout.

`billow` allows to define a [`BlockLayout`](struct.BlockLayout.html) which encodes
a SoA data layout. This layout can be used to subdivide user allocated memory blocks
in a tight and aligned fashion.

## Struct of Arrays

Struct of Arrays (SoA) describes a deinterleaved memory layout of struct fields.
Each array has the same number of elements. This layout is usually better suited for SIMD operations,

```ignore
+-----+-----+-----+-----
|  A  |  A  |  A  | ...
+-----+-----+-----+-----
+-------+-------+-------+-----
|   B   |   B   |   B   | ...
+-------+-------+-------+-----
+---+---+---+-----
| C | C | C | ...
+---+---+---+-----
```

## Examples

Allocating an aligned memory block from the system allocator and define a layout for the
following struct in SoA layout:

```rust
type Transform = [[f32; 4]; 4];
type Velocity = [f32; 3];

struct Block<'a> {
    transforms: &'a mut [Transform],
    velocity: &'a mut [Velocity],
}
```

```rust
# use norse_billow as billow;
# use std::alloc::{self, Layout, LayoutErr};
# use std::ptr::NonNull;
# type Transform = [[f32; 4]; 4];
# type Velocity = [f32; 3];
# fn main() -> Result<(), LayoutErr> {
const NUM_ELEMENTS: usize = 128;

// Define SoA layout.
let mut layout = billow::BlockLayout::build();
let transform_id = layout.add::<Transform>();
let velocity_id = layout.add::<Velocity>();
let block_layout = layout.finish();

// Allocate memory block for holding the elements.
let layout = block_layout.layout();
let size = layout.size() * NUM_ELEMENTS;
let memory = unsafe {
    alloc::alloc(Layout::from_size_align(size, layout.align())?)
};

let block = block_layout.apply(NonNull::new(memory).unwrap(), layout.size() * 128);
assert_eq!(block.len(), NUM_ELEMENTS);

// Get struct fields.
let transforms = unsafe { block.as_slice::<Transform>(transform_id) };
let velocities = unsafe { block.as_slice::<Velocity>(velocity_id) };

assert_eq!(transforms.len(), velocities.len());
# Ok(())
# }
```
*/

use indexmap::IndexMap;
use std::alloc::Layout;
use std::ops::Range;
use std::ptr::NonNull;
use std::slice;

/// Unique handle for an array field in a layout definition.
pub type LayoutSlot = usize;

/// Layout builder
pub struct LayoutBuilder {
    layouts: Vec<(LayoutSlot, Layout)>,
    max_alignment: usize,
    element_size: usize,
}

impl LayoutBuilder {
    /// Add a new typed compoent to the layout.
    ///
    /// Returns an unique handle for this layout, which can be used for
    /// retrieving the corresponding slice when applied to a memory range.
    ///
    /// The same type may be added multiple times to the same layout. Each addition
    /// will result in a new slot and allocate a slice in the memory region.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use norse_billow::BlockLayout;
    /// struct Foo {
    ///     a: usize,
    ///     b: [f32; 2],
    /// }
    ///
    /// let mut layout = BlockLayout::build();
    /// let handle_foo = layout.add::<Foo>();
    /// let handle_u8_0 = layout.add::<u8>();
    /// let handle_u8_1 = layout.add::<u8>();
    ///
    /// assert_ne!(handle_u8_0, handle_u8_1);
    /// ```
    pub fn add<T>(&mut self) -> LayoutSlot {
        let layout = Layout::new::<T>();
        self.max_alignment = self.max_alignment.max(layout.align());
        self.element_size += layout.size();

        let slot = self.layouts.len();
        self.layouts.push((slot, layout));
        slot
    }

    /// Bake the layout scheme into a finalized block layout.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use norse_billow::BlockLayout;
    ///
    /// let block_layout = {
    ///     let mut layout = BlockLayout::build();
    ///     layout.add::<[f32; 4]>();
    ///     layout.finish()
    /// };
    /// ```
    pub fn finish(mut self) -> BlockLayout {
        // Sort layouts to match our scheme (descending alignment).
        self.layouts
            .sort_by(|(slot_a, layout_a), (slot_b, layout_b)| {
                layout_a
                    .align()
                    .cmp(&layout_b.align())
                    .reverse()
                    .then(slot_a.cmp(slot_b))
            });
        &self.layouts;
        let slot_map = self
            .layouts
            .iter()
            .enumerate()
            .map(|(i, (slot, _))| (*slot, i))
            .collect();
        &slot_map;
        let sub_layouts = self.layouts.into_iter().map(|(_, layout)| layout).collect();
        let layout = Layout::from_size_align(self.element_size, self.max_alignment).unwrap();

        BlockLayout {
            slot_map,
            layout,
            sub_layouts,
        }
    }
}

/// SoA layout definition
///
/// ## Layout
///
/// The [`LayoutBuilder`](struct.LayoutBuilder.html) will reorder the components
/// depending on their alignment to avoid padding. The order is deterministic.
///
/// The components will be order by alignment first (descending) and by insertion order
/// for equal alignments. The resulting block layout will be aligned to the largest
/// alignment of all components. Due to enforced power-of-two alignments for all layouts
/// all components will be aligned and tightly packed.
pub struct BlockLayout {
    slot_map: IndexMap<LayoutSlot, usize>,
    layout: Layout,
    sub_layouts: Vec<Layout>,
}

impl BlockLayout {
    /// Build a new block layout.
    pub fn build() -> LayoutBuilder {
        LayoutBuilder {
            layouts: Vec::new(),
            max_alignment: 1,
            element_size: 0,
        }
    }

    /// Returns the layout for a single element.
    ///
    /// This layout can be repeated to get the memory requirements for a specific number of elements.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Apply the block layout to a memory region.
    pub fn apply(&self, data: NonNull<u8>, size: usize) -> Block {
        if self.sub_layouts.is_empty() {
            return Block {
                range: 0..0,
                len: 0,
                slices: Vec::new(),
            };
        }

        assert_eq!(self.layout.align() & (self.layout.align() - 1), 0); // alignment must be power-of-two

        let ptr = data.as_ptr();

        let start = (ptr as usize + self.layout.align() - 1) & !(self.layout.align() - 1);
        let end = (ptr as usize + size) & !(self.layout.align() - 1);

        let initial_offset = start - ptr as usize;
        let size_aligned = end - start;
        let len = if self.layout.size() == 0 {
            !0
        } else {
            size_aligned / self.layout.size()
        };

        let mut offset = 0;
        let mut offsets = Vec::with_capacity(self.sub_layouts.len());

        for layout in &self.sub_layouts {
            assert_eq!(offset % layout.align(), 0);
            offsets.push(offset);
            offset += layout.size() * len;
        }

        let mut slices = Vec::with_capacity(self.sub_layouts.len());
        for slot in self.slot_map.values() {
            let offset = offsets[*slot];
            slices.push(NonNull::new(unsafe { (start as *mut u8).offset(offset as _) }).unwrap());
        }

        Block {
            range: initial_offset..initial_offset + size_aligned,
            len,
            slices,
        }
    }
}

/// Laid out memory block
pub struct Block {
    /// Memory range occupied by the block (offset).
    range: Range<usize>,

    /// Number of elements per slice.
    len: usize,

    /// Aligned pointers at the beginning of each slice.
    slices: Vec<NonNull<u8>>,
}

impl Block {
    //// Returns the offset range which denotes the occupied memory block.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Returns the number of elements in each individual array slice.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Get the raw pointer and len for a component slot.
    ///
    /// # Unsafe
    ///
    /// The type `T` **must** match the type used on `add` for the passed slot.
    ///
    /// # Panics
    ///
    /// `slot` must be a valid value obtained by the corresponding block layout.
    pub unsafe fn as_raw<T>(&self, slot: LayoutSlot) -> (*mut T, usize) {
        let slice = &self.slices[slot];
        (slice.cast::<T>().as_ptr(), self.len)
    }

    /// Get the mutable slice for a component slot.
    ///
    /// # Unsafe
    ///
    /// The type `T` **must** match the type used on `add` for the passed slot.
    /// All values in the resulting slice are undefined!
    ///
    /// # Panics
    ///
    /// `slot` must be a valid value obtained by the corresponding block layout.
    pub unsafe fn as_slice<T: Copy>(&self, slot: LayoutSlot) -> &mut [T] {
        let slice = &self.slices[slot];
        slice::from_raw_parts_mut(slice.cast::<T>().as_ptr(), self.len)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn empty() {
        let layout = BlockLayout::build().finish();
        let mut block = [0; 32];
        layout.apply(NonNull::new(block.as_mut_ptr()).unwrap(), 32);
    }

    #[test]
    fn single_zst() {
        struct Foo;

        let (layout, foo) = {
            let mut layout = BlockLayout::build();
            let foo = layout.add::<Foo>();
            (layout.finish(), foo)
        };

        let mut data = [0; 32];
        let block = layout.apply(NonNull::new(data.as_mut_ptr()).unwrap(), 32);

        unsafe {
            block.as_raw::<Foo>(foo);
        }
    }

    #[test]
    fn ordering() {
        #[derive(Copy, Clone)]
        struct Small {
            _a: u8,
            _b: u8,
            _c: u8,
        }

        #[derive(Copy, Clone)]
        struct Large {
            _a: f32,
            _b: [u64; 8],
        }

        let (layout, small, large) = {
            let mut layout = BlockLayout::build();
            let small = layout.add::<Small>();
            let large = layout.add::<Large>();
            (layout.finish(), small, large)
        };

        let mut data = [0; 512];
        let block = layout.apply(NonNull::new(data.as_mut_ptr()).unwrap(), 512);

        let small_layout = Layout::new::<Small>();
        let large_layout = Layout::new::<Large>();
        assert_eq!(
            layout.layout().align(),
            small_layout.align().max(large_layout.align())
        );
        assert_eq!(
            layout.layout().size(),
            small_layout.size() + large_layout.size()
        );

        unsafe {
            block.as_slice::<Small>(small);
            block.as_slice::<Large>(large);
        }
    }
}

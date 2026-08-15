//! Minimal reader for bootloader-provided Flattened Device Tree blobs.
//!
//! This module validates the FDT header and exposes a forward-only node
//! iterator. It is intentionally a small early-boot parser rather than a full
//! device-tree implementation: `reg` decoding supports common one- and
//! two-cell address/size pairs and compatible matching is string-based.

const FDT_MAGIC: u32 = 0xD00DFEED;

const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

/// Errors that can occur when validating or reading a DTB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtbError {
    /// The bootloader did not supply a DTB address.
    NullPointer,
    /// The FDT header magic was not `0xD00D_FEED`.
    InvalidMagic,
    /// A requested operation would address bytes outside the DTB.
    OutOfBounds,
    /// A requested node was not present in the tree.
    NodeNotFound,
}

/// Root handle representing a validated Device Tree in memory.
#[derive(Debug, Clone, Copy)]
pub struct Dtb<'a> {
    slice: &'a [u8],
    struct_off: usize,
    strings_off: usize,
}

impl<'a> Dtb<'a> {
    /// Creates a DTB view from a raw physical memory pointer passed by the bootloader.
    ///
    /// # Safety
    /// The caller must ensure `ptr` points to valid, accessible memory containing a DTB.
    /// Its header's total-size field must describe a readable contiguous range.
    ///
    /// # Errors
    ///
    /// Returns [`DtbError::NullPointer`] for a zero pointer and
    /// [`DtbError::InvalidMagic`] if the header does not contain an FDT magic.
    pub unsafe fn from_ptr(ptr: usize) -> Result<Self, DtbError> {
        if ptr == 0 {
            return Err(DtbError::NullPointer);
        }

        let raw_ptr = ptr as *const u32;

        // Read magic (DTB values are big-endian)
        let magic = unsafe { u32::from_be(raw_ptr.add(0).read_unaligned()) };
        if magic != FDT_MAGIC {
            return Err(DtbError::InvalidMagic);
        }

        let total_size = unsafe { u32::from_be(raw_ptr.add(1).read_unaligned()) as usize };
        let struct_off = unsafe { u32::from_be(raw_ptr.add(2).read_unaligned()) as usize };
        let strings_off = unsafe { u32::from_be(raw_ptr.add(3).read_unaligned()) as usize };

        let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, total_size) };

        Ok(Self {
            slice,
            struct_off,
            strings_off,
        })
    }

    /// Finds the first device node compatible with `compatible`.
    ///
    /// A match is performed against the NUL-separated `compatible` property,
    /// such as `"arm,pl011"`.
    pub fn find_compatible(&self, compatible: &str) -> Option<Node<'a>> {
        for node in self.nodes() {
            if node.is_compatible(compatible) {
                return Some(node);
            }
        }
        None
    }

    /// Finds the first node whose name contains `name_part`.
    ///
    /// This is a substring search, not a path lookup; use it only when that
    /// looser matching behavior is intended.
    pub fn find_node(&self, name_part: &str) -> Option<Node<'a>> {
        for node in self.nodes() {
            if node.name().contains(name_part) {
                return Some(node);
            }
        }
        None
    }

    /// Returns an iterator that walks all begin-node tokens in tree order.
    pub fn nodes(&self) -> NodeIterator<'a> {
        NodeIterator {
            dtb: *self,
            curr_offset: self.struct_off,
        }
    }
}

/// A node in the Device Tree (for example, `/soc/uart@9000000`).
#[derive(Debug, Clone, Copy)]
pub struct Node<'a> {
    dtb: Dtb<'a>,
    name: &'a str,
    struct_offset: usize,
}

impl<'a> Node<'a> {
    /// Returns the node's local name, or `/` for the root node.
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Returns whether the node's `compatible` property contains `compat_str`.
    pub fn is_compatible(&self, compat_str: &str) -> bool {
        if let Some(prop) = self.property("compatible") {
            let target_bytes = compat_str.as_bytes();
            let val_bytes = prop.value();

            // Search for target byte sequence directly in the property value
            if val_bytes.len() >= target_bytes.len() {
                return val_bytes
                    .windows(target_bytes.len())
                    .any(|window| window == target_bytes);
            }
        }
        false
    }

    /// Looks up a property by its exact name on this node.
    pub fn property(&self, target_name: &str) -> Option<Property<'a>> {
        let mut curr = self.struct_offset;

        loop {
            if curr + 4 > self.dtb.slice.len() {
                break;
            }

            let token = read_be_u32(self.dtb.slice, curr);
            curr += 4;

            match token {
                FDT_PROP => {
                    let len = read_be_u32(self.dtb.slice, curr) as usize;
                    let name_off = read_be_u32(self.dtb.slice, curr + 4) as usize;
                    curr += 8;

                    let prop_name = self.dtb.read_string_at(self.dtb.strings_off + name_off)?;

                    if prop_name == target_name {
                        if curr + len > self.dtb.slice.len() {
                            return None;
                        }
                        let value_slice = &self.dtb.slice[curr..curr + len];
                        return Some(Property {
                            name: prop_name,
                            value: value_slice,
                        });
                    }

                    // Advance offset aligned to 4 bytes
                    curr += (len + 3) & !3;
                }
                FDT_NOP => {}
                FDT_BEGIN_NODE | FDT_END_NODE | FDT_END | _ => break,
            }
        }

        None
    }

    /// Parses the `reg` property as `(physical_base, size)`.
    /// Supports standard 64-bit addresses (`#address-cells = <2>`, `#size-cells = <2>`).
    pub fn reg(&self) -> Option<(u64, u64)> {
        let prop = self.property("reg")?;
        let val = prop.value();

        if val.len() >= 16 {
            // 64-bit address + 64-bit size
            let addr_hi = read_be_u32(val, 0) as u64;
            let addr_lo = read_be_u32(val, 4) as u64;
            let size_hi = read_be_u32(val, 8) as u64;
            let size_lo = read_be_u32(val, 12) as u64;

            let address = (addr_hi << 32) | addr_lo;
            let size = (size_hi << 32) | size_lo;

            Some((address, size))
        } else if val.len() >= 8 {
            // 32-bit address + 32-bit size
            let address = read_be_u32(val, 0) as u64;
            let size = read_be_u32(val, 4) as u64;

            Some((address, size))
        } else {
            None
        }
    }
}

/// A key-value property attached to a [`Node`].
#[derive(Debug, Clone, Copy)]
pub struct Property<'a> {
    name: &'a str,
    value: &'a [u8],
}

impl<'a> Property<'a> {
    /// Returns the property name from the DTB string table.
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Returns the raw, big-endian property bytes.
    pub fn value(&self) -> &'a [u8] {
        self.value
    }

    /// Interprets the property value as UTF-8, removing one trailing NUL byte.
    pub fn as_str(&self) -> Option<&'a str> {
        let bytes = if self.value.ends_with(&[0]) {
            &self.value[..self.value.len() - 1]
        } else {
            self.value
        };
        core::str::from_utf8(bytes).ok()
    }

    /// Interprets the first four property bytes as a big-endian `u32`.
    pub fn as_u32(&self) -> Option<u32> {
        if self.value.len() >= 4 {
            Some(read_be_u32(self.value, 0))
        } else {
            None
        }
    }

    /// Interprets the first eight property bytes as a big-endian `u64`.
    pub fn as_u64(&self) -> Option<u64> {
        if self.value.len() >= 8 {
            let hi = read_be_u32(self.value, 0) as u64;
            let lo = read_be_u32(self.value, 4) as u64;
            Some((hi << 32) | lo)
        } else {
            None
        }
    }
}

/// Iterator that yields nodes in Flattened Device Tree structure-block order.
pub struct NodeIterator<'a> {
    dtb: Dtb<'a>,
    curr_offset: usize,
}

impl<'a> Iterator for NodeIterator<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.curr_offset + 4 <= self.dtb.slice.len() {
            let token = read_be_u32(self.dtb.slice, self.curr_offset);
            self.curr_offset += 4;

            match token {
                FDT_BEGIN_NODE => {
                    let name_start = self.curr_offset;
                    let mut name_len = 0;

                    while self.curr_offset < self.dtb.slice.len()
                        && self.dtb.slice[self.curr_offset] != 0
                    {
                        name_len += 1;
                        self.curr_offset += 1;
                    }

                    // Skip null terminator
                    self.curr_offset += 1;
                    // 4-byte align
                    self.curr_offset = (self.curr_offset + 3) & !3;

                    let raw_name = &self.dtb.slice[name_start..name_start + name_len];
                    let name = core::str::from_utf8(raw_name).unwrap_or("");
                    let node_name = if name.is_empty() { "/" } else { name };

                    let struct_offset = self.curr_offset;

                    return Some(Node {
                        dtb: self.dtb,
                        name: node_name,
                        struct_offset,
                    });
                }
                FDT_PROP => {
                    let len = read_be_u32(self.dtb.slice, self.curr_offset) as usize;
                    self.curr_offset += 8; // skip len & name_off
                    self.curr_offset += (len + 3) & !3; // skip value & align
                }
                FDT_END_NODE | FDT_NOP => {}
                FDT_END | _ => break,
            }
        }

        None
    }
}

// Private Helpers

impl<'a> Dtb<'a> {
    fn read_string_at(&self, offset: usize) -> Option<&'a str> {
        if offset >= self.slice.len() {
            return None;
        }

        let mut len = 0;
        while offset + len < self.slice.len() && self.slice[offset + len] != 0 {
            len += 1;
        }

        core::str::from_utf8(&self.slice[offset..offset + len]).ok()
    }
}

#[inline(always)]
fn read_be_u32(slice: &[u8], offset: usize) -> u32 {
    let bytes = [
        slice[offset],
        slice[offset + 1],
        slice[offset + 2],
        slice[offset + 3],
    ];
    u32::from_be_bytes(bytes)
}

impl hal::DeviceTree for Dtb<'_> {
    fn compatible_address(&self, compatible: &str) -> Option<usize> {
        Dtb::find_compatible(self, compatible)
            .and_then(|node| node.reg().map(|(base, _)| base as usize))
    }
}

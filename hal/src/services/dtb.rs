//! Minimal reader for bootloader-provided Flattened Device Tree blobs.

use crate::services::addr::{PhysAddr, PhysRange};

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
    /// Creates a DTB view from a raw physical memory pointer
    ///
    /// # Safety
    /// The caller must ensure `ptr` points to valid, accessible memory containing a DTB.
    /// Its header's total-size field must describe a readable contiguous range.
    ///
    /// # Errors
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

    /// Writes the RAM regions the tree describes into `out`; returns how many.
    ///
    /// These are the `reg` entries of the root's `memory` nodes (every entry
    /// of every node), so it works before the heap exists.
    pub fn memory_regions(&self, out: &mut [PhysRange]) -> usize {
        let mut count = 0;
        self.walk(|walked| {
            if walked.depth() != 1 || !walked.is_enabled() {
                return;
            }
            let node = walked.node();
            let is_memory = match node.property("device_type").and_then(|p| p.as_str()) {
                Some(kind) => kind == "memory",
                // Old trees leave `device_type` out; `memory` or `memory@<address>`.
                None => node.name() == "memory" || node.name().starts_with("memory@"),
            };
            if !is_memory {
                return;
            }
            walked.for_each_reg(|base, size| {
                if size > 0 && count < out.len() {
                    out[count] = PhysRange::new(PhysAddr::new(base as usize), size as usize);
                    count += 1;
                }
            });
        });
        count
    }

    /// Visits every node in tree order, with what is needed to read its `reg`
    /// the way the CPU sees it. Does not allocate.
    pub fn walk(&self, mut visit: impl FnMut(&WalkedNode<'_, 'a>)) {
        let slice = self.slice;
        let mut frames = [Frame::DEFAULT; MAX_DEPTH];
        let mut depth = 0usize;
        // Nodes nested deeper than `MAX_DEPTH`, which are not visited.
        let mut too_deep = 0usize;
        let mut offset = self.struct_off;

        while offset + 4 <= slice.len() {
            let token = read_be_u32(slice, offset);
            offset += 4;
            match token {
                FDT_BEGIN_NODE => {
                    let mut end = offset;
                    while end < slice.len() && slice[end] != 0 {
                        end += 1;
                    }
                    let name = core::str::from_utf8(&slice[offset..end]).unwrap_or("");
                    offset = (end + 1 + 3) & !3;

                    if depth == MAX_DEPTH {
                        too_deep += 1;
                        continue;
                    }

                    let node = Node { dtb: *self, name: if name.is_empty() { "/" } else { name }, struct_offset: offset };
                    let parent_enabled = depth == 0 || frames[depth - 1].enabled;
                    frames[depth] = Frame {
                        addr_cells: node.property("#address-cells").and_then(|p| p.as_u32()).unwrap_or(2),
                        size_cells: node.property("#size-cells").and_then(|p| p.as_u32()).unwrap_or(1),
                        ranges: node.property("ranges").map(|p| p.value()),
                        enabled: parent_enabled && node.is_enabled(),
                        interrupt_parent: node
                            .property("interrupt-parent")
                            .and_then(|p| p.as_u32())
                            .or(if depth == 0 { None } else { frames[depth - 1].interrupt_parent }),
                    };
                    visit(&WalkedNode { node, depth, frames: &frames[..=depth] });
                    depth += 1;
                }
                FDT_END_NODE => {
                    if too_deep > 0 {
                        too_deep -= 1;
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
                FDT_PROP => {
                    if offset + 8 > slice.len() {
                        break;
                    }
                    let len = read_be_u32(slice, offset) as usize;
                    offset += 8 + ((len + 3) & !3);
                }
                FDT_NOP => {}
                _ => break,
            }
        }
    }

    /// Returns an iterator that walks all begin-node tokens in tree order.
    pub fn nodes(&self) -> NodeIterator<'a> {
        NodeIterator {
            dtb: *self,
            curr_offset: self.struct_off,
        }
    }

    /// Returns the raw device path string assigned to `alias` in the `/aliases` node.
    pub fn resolve_alias(&self, alias: &str) -> Option<&'a str> {
        self.nodes()
            .find(|node| node.name() == "aliases")
            .and_then(|aliases_node| aliases_node.property(alias))
            .and_then(|prop| prop.as_str())
    }

    /// Finds the node that matches the given `phandle`
    pub fn find_phandle(&self, phandle: u32) -> Option<Node<'a>> {
        if phandle == 0 || phandle == u32::MAX {
            return None;
        }

        self.nodes().find(|node| node.phandle() == Some(phandle))
    }

    /// Finds a node by its full device tree path (`/soc/uart@9000000`).
    ///
    /// A component may leave out the unit address (`/soc/uart`), as the
    /// specification allows.
    pub fn find_path(&self, path: &str) -> Option<Node<'a>> {
        let path = path.trim_end_matches('\0');
        if !path.starts_with('/') {
            return None;
        }

        let mut wanted = [""; MAX_DEPTH];
        let mut count = 0;
        for component in path.split('/').filter(|component| !component.is_empty()) {
            *wanted.get_mut(count)? = component;
            count += 1;
        }

        let mut names = [""; MAX_DEPTH];
        let mut found = None;
        self.walk(|walked| {
            let depth = walked.depth();
            let node = walked.node();
            names[depth] = node.name();
            if found.is_none()
                && depth == count
                && (1..=depth).all(|level| path_component_matches(wanted[level - 1], names[level]))
            {
                found = Some(node);
            }
        });
        found
    }

    /// The node `/chosen/stdout-path` names: the device the firmware used as
    /// its console. The value is a path or an alias, optionally followed by
    /// `:` and serial settings, which are not our business.
    pub fn stdout_node(&self) -> Option<Node<'a>> {
        let chosen = self.nodes().find(|node| node.name() == "chosen")?;
        let value = chosen
            .property("stdout-path")
            .or_else(|| chosen.property("linux,stdout-path"))?
            .as_str()?;
        let path = value.split(':').next()?;
        if path.starts_with('/') { self.find_path(path) } else { self.find_alias(path) }
    }

    /// Finds node by alias
    pub fn find_alias(&self, alias: &str) -> Option<Node<'a>> {
        let path = self.resolve_alias(alias)?;
        self.find_path(path)
    }

    /// The range the blob itself occupies.
    pub fn physical_range(&self) -> PhysRange {
        PhysRange::new(PhysAddr::new(self.slice.as_ptr() as usize), self.slice.len())
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

    /// The tree the node belongs to.
    pub fn dtb(&self) -> Dtb<'a> {
        self.dtb
    }

    /// Where the node starts in the structure block; unique per node, so two
    /// handles are the same node exactly when their offsets are equal.
    pub fn offset(&self) -> usize {
        self.struct_offset
    }

    /// The node's `compatible` strings, most specific first.
    pub fn compatible(&self) -> impl Iterator<Item = &'a str> + use<'a> {
        self.property("compatible")
            .map(|prop| prop.value())
            .unwrap_or(&[])
            .split(|byte| *byte == 0)
            .filter_map(|string| core::str::from_utf8(string).ok())
            .filter(|string| !string.is_empty())
    }

    /// Returns whether one of the node's `compatible` strings is exactly `compat_str`.
    pub fn is_compatible(&self, compat_str: &str) -> bool {
        self.compatible().any(|candidate| candidate == compat_str)
    }

    /// Whether `status` allows the node to be used (absent means yes).
    pub fn is_enabled(&self) -> bool {
        matches!(self.property("status").and_then(|prop| prop.as_str()), None | Some("okay") | Some("ok"))
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


            // the patterns r present for explicitness
            #[allow(clippy::wildcard_in_or_patterns)]
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

    /// Returns the node's phandle, if it has one
    ///
    /// This method checks both the standard `phandle` and
    /// the legacy `linux,phandle`. Returns result as a `u32`
    pub fn phandle(&self) -> Option<u32> {
        self.property("phandle")
            .or_else(|| self.property("linux,phandle"))
            .and_then(|prop| prop.as_u32())
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

            // the patterns r present for explicitness
            #[allow(clippy::wildcard_in_or_patterns)]
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

// -------------------------------- Walking --------------------------------

/// Most nested levels [`Dtb::walk`] follows; deeper nodes are skipped.
const MAX_DEPTH: usize = 24;

/// What a node says about the space its children live in.
#[derive(Clone, Copy)]
struct Frame<'a> {
    /// `#address-cells` (2 if the node does not say).
    addr_cells: u32,
    /// `#size-cells` (1 if the node does not say).
    size_cells: u32,
    /// `ranges`: `None` if absent, which means the children's addresses are
    /// not CPU addresses (an I2C bus, say); empty means they are the same.
    ranges: Option<&'a [u8]>,
    /// This node and every node above it are enabled.
    enabled: bool,
    /// The phandle in effect as `interrupt-parent`: the node's own, else the
    /// nearest ancestor's.
    interrupt_parent: Option<u32>,
}

impl Frame<'_> {
    const DEFAULT: Frame<'static> = Frame { addr_cells: 2, size_cells: 1, ranges: None, enabled: true, interrupt_parent: None };
}

/// A node handed out by [`Dtb::walk`].
pub struct WalkedNode<'w, 'a> {
    node: Node<'a>,
    depth: usize,
    /// The frames of the node and all its ancestors, the root first.
    frames: &'w [Frame<'a>],
}

impl<'a> WalkedNode<'_, 'a> {
    pub fn node(&self) -> Node<'a> {
        self.node
    }

    /// How many ancestors the node has (the root has none).
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Whether the node, and everything above it, has an enabled `status`.
    pub fn is_enabled(&self) -> bool {
        self.frames[self.depth].enabled
    }

    /// The phandle of the node's interrupt controller, if the node or an
    /// ancestor names one with `interrupt-parent`.
    pub fn interrupt_parent(&self) -> Option<u32> {
        self.frames[self.depth].interrupt_parent
    }

    /// Calls `f(address, size)` for each `reg` entry, with `address` as the CPU
    /// sees it (translated through the `ranges` of every bus above).
    ///
    /// Returns `false`, calling `f` for nothing, if the node's `reg` is not
    /// memory the CPU can reach: it sits on a bus without `ranges`, or uses
    /// cell sizes this reader does not support. A node without `reg` returns
    /// `true`.
    pub fn for_each_reg(&self, mut f: impl FnMut(u64, u64)) -> bool {
        let Some(parent) = self.depth.checked_sub(1).map(|parent| &self.frames[parent]) else {
            return false;
        };
        let Some(reg) = self.node.property("reg") else {
            return true;
        };
        let (addr_cells, size_cells) = (parent.addr_cells as usize, parent.size_cells as usize);
        if addr_cells == 0 || addr_cells > 2 || size_cells > 2 {
            return false;
        }
        if self.frames[1..self.depth].iter().any(|bus| bus.ranges.is_none()) {
            return false;
        }

        let value = reg.value();
        let words = addr_cells + size_cells;
        let mut entry = 0;
        while (entry + 1) * words * 4 <= value.len() {
            let address = cells_at(value, entry * words, addr_cells);
            let size = cells_at(value, entry * words + addr_cells, size_cells);
            let (Some(address), Some(size)) = (address, size) else { return false };
            match self.translate(address, size) {
                Some(address) => f(address, size),
                None => return false,
            }
            entry += 1;
        }
        true
    }

    /// Carries a bus address up through every `ranges` to a CPU address.
    fn translate(&self, mut address: u64, size: u64) -> Option<u64> {
        for level in (1..self.depth).rev() {
            let bus = &self.frames[level];
            let ranges = bus.ranges?;
            if ranges.is_empty() {
                continue;
            }
            let child_cells = bus.addr_cells as usize;
            let parent_cells = self.frames[level - 1].addr_cells as usize;
            let size_cells = bus.size_cells as usize;
            if child_cells > 2 || parent_cells > 2 || size_cells > 2 {
                return None;
            }

            let words = child_cells + parent_cells + size_cells;
            let mut mapped = None;
            let mut entry = 0;
            while words > 0 && (entry + 1) * words * 4 <= ranges.len() {
                let at = entry * words;
                let child = cells_at(ranges, at, child_cells)?;
                let parent = cells_at(ranges, at + child_cells, parent_cells)?;
                let length = cells_at(ranges, at + child_cells + parent_cells, size_cells)?;
                if address >= child && address - child < length && size <= length - (address - child) {
                    mapped = Some(address - child + parent);
                    break;
                }
                entry += 1;
            }
            address = mapped?;
        }
        Some(address)
    }
}

/// Reads `cells` (0, 1 or 2) big-endian words starting at word `word`.
fn cells_at(bytes: &[u8], word: usize, cells: usize) -> Option<u64> {
    let read = |index: usize| -> Option<u64> {
        let at = index.checked_mul(4)?;
        let raw = bytes.get(at..at + 4)?;
        Some(u64::from(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])))
    };
    match cells {
        0 => Some(0),
        1 => read(word),
        2 => Some((read(word)? << 32) | read(word + 1)?),
        _ => None,
    }
}

/// Whether path component `wanted` names the node called `name`. A component
/// without a unit address matches a node with any.
fn path_component_matches(wanted: &str, name: &str) -> bool {
    wanted == name || (!wanted.contains('@') && name.split('@').next() == Some(wanted))
}

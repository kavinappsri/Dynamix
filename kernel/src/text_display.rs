//! Text output on a framebuffer.
//!
//! [`TextDisplay`] is a tiny terminal that implements [`OutputStream`]

use hal::services::sync::IrqSpinLock;
use hal::Framebuffer;

use crate::font::{self, GLYPH_HEIGHT, GLYPH_WIDTH};
use crate::io::OutputStream;

/// Colors are `0x00RRGGBB`; the framebuffer's pixel format is applied by the HAL.
const FOREGROUND: u32 = 0x00_E0_E0_E0;
const BACKGROUND: u32 = 0x00_00_00_00;

/// Glyphs are drawn at an integer multiple of their 8x8 size, so text stays
/// readable on large screens. The scale is picked from the screen size, up to this.
const MAX_SCALE: usize = 4;
const MAX_CELL_PIXELS: usize = (GLYPH_WIDTH * MAX_SCALE) * (GLYPH_HEIGHT * MAX_SCALE);

const TAB_WIDTH: usize = 8;

/// The text grid and cursor; [`TextDisplay`] puts a lock around it.
struct Grid<'a> {
    framebuffer: &'a dyn Framebuffer,
    /// Pixels per glyph pixel, in both directions.
    scale: usize,
    cols: usize,
    rows: usize,
    /// Pixel position of the top-left cell; centers the grid on the screen.
    origin_x: usize,
    origin_y: usize,
    /// Cursor cell. `col == cols` means the line is full and the next glyph
    /// wraps to the next line (so a full line does not scroll early).
    col: usize,
    row: usize,
}

impl<'a> Grid<'a> {
    /// `None` if the framebuffer is too small to hold a single character cell.
    fn new(framebuffer: &'a dyn Framebuffer) -> Option<Self> {
        let (width, height) = framebuffer.dimensions();

        // At least 80 columns and 25 rows wherever the screen allows it.
        let scale = (width / 640).min(height / 400).clamp(1, MAX_SCALE);
        let (cell_width, cell_height) = (GLYPH_WIDTH * scale, GLYPH_HEIGHT * scale);
        let (cols, rows) = (width / cell_width, height / cell_height);
        if cols == 0 || rows == 0 {
            return None;
        }

        let mut display = Self {
            framebuffer,
            scale,
            cols,
            rows,
            origin_x: (width - cols * cell_width) / 2,
            origin_y: (height - rows * cell_height) / 2,
            col: 0,
            row: 0,
        };
        display.clear();
        Some(display)
    }

    fn clear(&mut self) {
        self.framebuffer.fill(BACKGROUND);
        self.col = 0;
        self.row = 0;
    }

    /// Interprets one byte of terminal output.
    ///
    /// Printable ASCII is drawn. `\n`, `\r`, backspace and tab move the
    /// cursor. UTF-8 sequences show up as a single `?`, and every other
    /// control byte is ignored.
    fn put_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            0x08 => self.backspace(),
            b'\t' => loop {
                self.put_glyph(b' ');
                if self.col % TAB_WIDTH == 0 {
                    break;
                }
            },
            0x20..=0x7E => self.put_glyph(byte),
            // Continuation bytes of a UTF-8 sequence; its first byte already drew the `?`.
            0x80..=0xBF => {}
            0xC0..=0xFF => self.put_glyph(b'?'),
            _ => {}
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        self.row += 1;
        if self.row >= self.rows {
            self.clear();
        }
    }

    /// Moves the cursor one cell back, without erasing. A terminal's erase is
    /// backspace, space, backspace, which is what the console's line editing sends.
    ///
    /// At the start of a line it goes to the end of the line above, because
    /// the only way to be there is that the line above wrapped.
    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.cols - 1;
        }
    }

    fn put_glyph(&mut self, byte: u8) {
        if self.col >= self.cols {
            self.newline();
        }
        if let Some(glyph) = font::glyph(byte) {
            self.draw_cell(glyph);
        }
        self.col += 1;
    }

    /// Draws `glyph` in the cursor's cell, background included, so whatever
    /// was there before is replaced.
    fn draw_cell(&self, glyph: &[u8; GLYPH_HEIGHT]) {
        let scale = self.scale;
        let (cell_width, cell_height) = (GLYPH_WIDTH * scale, GLYPH_HEIGHT * scale);

        let mut cell = [BACKGROUND; MAX_CELL_PIXELS];
        for (glyph_y, &bits) in glyph.iter().enumerate() {
            for glyph_x in 0..GLYPH_WIDTH {
                // Bit 0 is the leftmost pixel of the row.
                if (bits >> glyph_x) & 1 == 0 {
                    continue;
                }
                for dy in 0..scale {
                    let start = (glyph_y * scale + dy) * cell_width + glyph_x * scale;
                    cell[start..start + scale].fill(FOREGROUND);
                }
            }
        }

        let x = self.origin_x + self.col * cell_width;
        let y = self.origin_y + self.row * cell_height;
        // The cell lies inside the grid, which lies inside the screen, so this cannot fail.
        let _ = self.framebuffer.blit_xrgb8888(x, y, &cell[..cell_width * cell_height], cell_width, cell_height);
    }
}

/// A [`Framebuffer`] used as a text screen.
///
/// Writing to it needs no setup beyond [`TextDisplay::new`], and it can be
/// written to from anywhere: the cursor state sits behind an interrupt-safe lock.
pub struct TextDisplay<'a> {
    grid: IrqSpinLock<Grid<'a>>,
}

impl<'a> TextDisplay<'a> {
    /// Takes over `framebuffer` as a text screen and clears it. `None` if it is
    /// too small to hold a single character cell.
    pub fn new(framebuffer: &'a dyn Framebuffer) -> Option<Self> {
        Some(Self { grid: IrqSpinLock::new(Grid::new(framebuffer)?) })
    }

    /// Clears the whole screen and moves the cursor to the top-left corner.
    pub fn clear(&self) {
        self.grid.lock().clear();
    }
}

impl OutputStream for TextDisplay<'_> {
    fn write_byte(&self, byte: u8) {
        self.grid.lock().put_byte(byte);
    }

    /// Takes the lock once for the whole string instead of per byte.
    fn write_str(&self, text: &str) {
        let mut grid = self.grid.lock();
        for byte in text.bytes() {
            grid.put_byte(byte);
        }
    }
}
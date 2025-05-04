use core::{fmt, ptr};
use noto_sans_mono_bitmap::{FontWeight, get_raster, RasterizedChar};
use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};
use noto_sans_mono_bitmap::RasterHeight::Size16;
use kernel::RacyCell;

static WRITER: RacyCell<Option<ScreenWriter>> = RacyCell::new(None);
pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let writer = unsafe { WRITER.get_mut() }.as_mut().unwrap();
        writer.write_str(s)
    }
}

pub fn screenwriter() -> &'static mut ScreenWriter {
    unsafe { WRITER.get_mut() }.as_mut().unwrap()
}

pub fn init(buffer: &'static mut FrameBuffer) {
    let info = buffer.info();
    let framebuffer = buffer.buffer_mut();
    let writer = ScreenWriter::new(framebuffer, info);
    *unsafe { WRITER.get_mut() } = Some(writer);
}

const LINE_SPACING: usize = 2; // Increased line spacing for better readability

pub struct ScreenWriter {
    framebuffer: &'static mut [u8],
    info: FrameBufferInfo,
    x_pos: usize,
    y_pos: usize,
}

impl ScreenWriter {
    pub fn new(framebuffer: &'static mut [u8], info: FrameBufferInfo) -> Self {
        let mut logger = Self {
            framebuffer,
            info,
            x_pos: 0,
            y_pos: 0,
        };
        logger.clear();
        logger
    }

    fn newline(&mut self) {
        self.y_pos += Size16 as usize + LINE_SPACING;
        self.carriage_return()
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    pub fn clear(&mut self) {
        self.x_pos = 0;
        self.y_pos = 0;
        self.framebuffer.fill(0);
    }

    pub fn clear_screen(&mut self, r: u8, g: u8, b: u8) {
        for y in 0..self.height() {
            for x in 0..self.width() {
                self.safe_draw_pixel(x, y, r, g, b);
            }
        }
    }

    pub fn width(&self) -> usize {
        self.info.width as usize
    }

    pub fn height(&self) -> usize {
        self.info.height as usize
    }

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                if let Some(bitmap_char) = get_raster(c, FontWeight::Bold, Size16) { // Changed to Bold
                    if self.x_pos + bitmap_char.width() > self.width() {
                        self.newline();
                    }
                    if self.y_pos + bitmap_char.height() > self.height() {
                        self.clear();
                    }
                    self.write_rendered_char(bitmap_char);
                }
            }
        }
    }

    pub fn safe_draw_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        if x >= self.width() || y >= self.height() {
            return;
        }
        
        let pixel_offset = y * self.info.stride as usize + x;
        let color = match self.info.pixel_format {
            PixelFormat::Rgb => [r, g, b, 0],
            PixelFormat::Bgr => [b, g, r, 0],
            other => {
                self.info.pixel_format = PixelFormat::Rgb;
                panic!("pixel format {:?} not supported", other)
            }
        };
        
        let bytes_per_pixel = self.info.bytes_per_pixel as usize;
        let byte_offset = pixel_offset * bytes_per_pixel;
        
        if byte_offset + bytes_per_pixel <= self.framebuffer.len() {
            self.framebuffer[byte_offset..(byte_offset + bytes_per_pixel)]
                .copy_from_slice(&color[..bytes_per_pixel]);
        }
    }

    pub fn draw_char(&mut self, x: usize, y: usize, c: char, r: u8, g: u8, b: u8) {
        if let Some(bitmap_char) = get_raster(c, FontWeight::Bold, Size16) { // Changed to Bold
            for (char_y, row) in bitmap_char.raster().iter().enumerate() {
                for (char_x, &intensity) in row.iter().enumerate() {
                    if intensity > 0 {
                        self.safe_draw_pixel(x + char_x, y + char_y, r, g, b);
                    }
                }
            }
        }
    }

    pub fn draw_string(&mut self, x: usize, y: usize, text: &str, r: u8, g: u8, b: u8) {
        let mut x_pos = x;
        for c in text.chars() {
            self.draw_char(x_pos, y, c, r, g, b);
            x_pos += 9; // Increased character spacing
        }
    }

    pub fn draw_string_centered(&mut self, y: usize, text: &str, r: u8, g: u8, b: u8) {
        let x = (self.width() - text.len() * 9) / 2; // Adjusted for new character spacing
        self.draw_string(x, y, text, r, g, b);
    }

    fn write_rendered_char(&mut self, rendered_char: RasterizedChar) {
        for (y, row) in rendered_char.raster().iter().enumerate() {
            for (x, &byte) in row.iter().enumerate() {
                self.safe_draw_pixel(
                    self.x_pos + x, 
                    self.y_pos + y,
                    byte / 2, // Changed color formula
                    byte,
                    byte / 1
                );
            }
        }
        self.x_pos += rendered_char.width();
    }
}

unsafe impl Send for ScreenWriter {}
unsafe impl Sync for ScreenWriter {}

impl fmt::Write for ScreenWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}
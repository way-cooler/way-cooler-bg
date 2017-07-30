//! Colors used for drawing to the screen
//! TODO This needs to be in a helper library

use std::convert::From;

/// Color to draw to the screen, including the alpha channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8
}


impl Color {
    /// Makes a new solid color, with no transparency.
    pub fn solid_color(red: u8, green: u8, blue: u8) -> Self {
        Color {
            red,
            green,
            blue,
            alpha: 255
        }
    }

    /// Gets the values of the colors, in this order:
    /// (Red, Green, Blue, Alpha)
    #[allow(dead_code)]
    pub fn to_u8s(self) -> (u8, u8, u8, u8) {
        (self.red, self.green, self.blue, self.alpha)
    }

    pub fn from_u8s(red: u8, green: u8, blue: u8) -> Self {
        Color::solid_color(red, green, blue)
    }

    /// To a u32, represented as in RGBA format
    pub fn to_u32(self) -> u32 {
        let values = [self.red, self.green, self.blue, self.alpha];
        unsafe {
            ::std::mem::transmute(values)
        }
    }
}

impl From<u32> for Color {
    fn from(val: u32) -> Self {
        let blue = ((val & 0xff0000) >> 16) as u8;
        let green = ((val & 0x00ff00) >> 8) as u8;
        let red = (val & 0x0000ff) as u8;
        Color::solid_color(red, green, blue)
    }
}

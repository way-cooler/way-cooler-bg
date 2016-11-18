#[macro_use]
extern crate wayland_client;

extern crate tempfile;

extern crate byteorder;
extern crate image;

use std::env;
use std::process::exit;
use std::io::{BufReader};
use image::{load, ImageFormat};
use std::mem::transmute;
use std::os::unix::io::AsRawFd;
use std::io::Write;
use std::fs::File;

use wayland_client::wayland::get_display;
use wayland_client::wayland::compositor::{WlCompositor, WlSurface};
use wayland_client::wayland::shell::WlShell;
use wayland_client::wayland::shm::{WlShm, WlShmFormat};
use wayland_client::wayland::seat::{WlSeat, WlPointerEvent};
use wayland_client::cursor::load_theme;
use wayland_client::{EventIterator, Proxy};

use byteorder::{NativeEndian, WriteBytesExt};

wayland_env!(WaylandEnv,
             compositor: WlCompositor,
             shell: WlShell,
             shm: WlShm,
             seat: WlSeat
);

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
/// Holds the bytes to represent a colored background.
/// To be written into a wayland surface.
pub struct Color(pub [u8; 4]);

impl Color {
    /// Generate a new color out of a u32.
    /// E.G: 0xFFFFFF
    pub fn from_u32(color: u32) -> Self {
        unsafe { Color(transmute(color)) }
    }

    pub fn as_u32(&self) -> u32 {
        unsafe { transmute(self.0)}
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() < 3 {
        println!("Please supply either a file path or a color (written in hex)");
        println!("Please also supply a path to a cursor image");
        exit(1);
    }
    let input = &args[1];
    let cursor_path = &args[2];
    if let Ok(color) = input.parse::<u32>() {
        let color = Color::from_u32(color);
        generate_solid_background(color, cursor_path.clone());
    } else {
        generate_image_background(input.clone(), cursor_path.clone());
    }
}

fn weird_math(num: u8, third_num: u32) -> u8 {
    let big_num = num as u32;
    ((big_num * third_num) / 255) as u8
}

/// Given a solid color, writes bytes associated with that color to
/// a special Wayland surface which is then rendered as a background for Way Cooler.
pub fn generate_solid_background(color: Color, cursor_path: String) {
    // Get shortcuts to the globals.
    let (display, iter) = get_display()
        .expect("Unable to connect to a wayland compositor");
    let (env, evt_iter) = WaylandEnv::init(display, iter);
    let compositor = env.compositor.as_ref().map(|o| &o.0).unwrap();
    let shell = env.shell.as_ref().map(|o| &o.0).unwrap();
    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();

    // Create the surface we are going to write into
    let background_surface = compositor.create_surface();
    let shell_surface = shell.get_shell_surface(&background_surface);
    let mut tmp = tempfile::tempfile().ok().expect("Unable to create a tempfile.");

    // Calculate how big the buffer needs to be from the output resolution
    // TODO Get the output size from way cooler somehow...
    //let resolution = output.get_resolution()
    //    .expect("Couldn't get output resolution");
    //let (width, height) = (resolution.w as i32, resolution.h as i32);
    let width = 800; let height = 600;
    let size = (width * height) as i32;

    // Write in the color coding to the surface
    for _ in 0..size {
        unsafe {
            tmp.write_u32::<NativeEndian>(transmute(color.0))
                .expect("Could not write to file")
        }
    }
    tmp.flush()
        .expect("Could not flush buffer");

    // Create the buffer that is mem-mapped to the temp file descriptor
    let pool = shm.create_pool(tmp.as_raw_fd(), size);
    let buffer = pool.create_buffer(0, width, height, width, WlShmFormat::Argb8888);
    // Tell Way Cooler not to set put this in the tree, treat as background
    shell_surface.set_class("Background".into());
    shell_surface.set_title(format!("0x{:x}", color.as_u32()));

    // Attach the buffer to the surface
    background_surface.attach(Some(&buffer), 0, 0);

    let cursor_surface = cursor_surface(cursor_path.as_str(), &env);
    main_background_loop(background_surface, cursor_surface, evt_iter, &env);
}

/// Given the data from an image, writes it to a special Wayland surface
/// which is then rendered as a background for Way Cooler.
pub fn generate_image_background(path: String, cursor_path: String) {
    let image_file = File::open(path.clone())
        .unwrap_or_else(|_| {
            println!("Could not open \"{:?}\"", path);
            panic!("Could not open image file");
        });
    let image_reader = BufReader::new(image_file);
    // TODO support more formats, split into separate function
    let image = load(image_reader, ImageFormat::PNG)
        .expect("Image was not a png file!");
    let mut image = image.to_rgba();
    let width = image.width();
    let height = image.height();
    let stride = width * 4;
    let size = stride * height;
    // TODO Split this into its own function
    {
        let pixels = image.enumerate_pixels_mut();
        for (_x, _y, pixel) in pixels {
            let alpha = pixel[3] as u32;
            pixel[0] = weird_math(pixel[0], alpha);
            pixel[1] = weird_math(pixel[1], alpha);
            pixel[2] = weird_math(pixel[2], alpha);

            let tmp = pixel[2];
            pixel[2] = pixel[0];
            pixel[0] = tmp;
        }
    }
    let vec = image.into_vec();
    let mut tmp = tempfile::NamedTempFile::new().expect("Unable to create a tempfile.");
    tmp.set_len(size as u64).expect("Could not truncate length of file");
    tmp.write_all(&*vec).unwrap();


    // TODO I want this shit outta here
    let (display, iter) = get_display()
        .expect("Unable to connect to a wayland compositor");
    let (env, evt_iter) = WaylandEnv::init(display, iter);
    let compositor = env.compositor.as_ref().map(|o| &o.0).unwrap();
    let shell = env.shell.as_ref().map(|o| &o.0).unwrap();
    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();

    // Create the surface we are going to write into
    let background_surface = compositor.create_surface();
    let shell_surface = shell.get_shell_surface(&background_surface);

    let pool = shm.create_pool(tmp.as_raw_fd(), size as i32);
    let background_buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, WlShmFormat::Argb8888);
    // Tell Way Cooler not to put this in the tree, treat as background
    // TODO Make this less hacky by actually giving way cooler access to this thing...
    shell_surface.set_class("Background".into());
    // TODO Actually give it the path or something idk
    shell_surface.set_title(format!("Image background yay"));

    // Attach the buffer to the surface
    background_surface.attach(Some(&background_buffer), 0, 0);
    background_surface.commit();
    background_surface.set_buffer_scale(1);
    background_surface.damage(0, 0, width as i32, height as i32);
    let cursor_surface = cursor_surface(cursor_path.as_str(), &env);
    main_background_loop(background_surface, cursor_surface, evt_iter, &env);

}

fn cursor_surface(cursor_path: &str, env: &WaylandEnv) -> WlSurface {
    let compositor = env.compositor.as_ref().map(|o| &o.0).unwrap();
    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();

    let cursor_surface = compositor.create_surface();
    let cursor_theme = load_theme(None, 16, shm);
    /* If the theme has a predefined cursor, just use that */
    if let Some(cursor) = cursor_theme.get_cursor("default") {
        let cursor_buffer = cursor.frame_buffer(0).expect("Couldn't get frame_buffer");
        cursor_surface.attach(Some(&*cursor_buffer), 0, 0);
    } else {
        let cursor_file = File::open(cursor_path.clone())
            .unwrap_or_else(|_| {
                println!("Could not open \"{:?}\"", cursor_path);
                panic!("Could not open image file");
            });
        let image_reader = BufReader::new(cursor_file);
        // TODO support more formats, split into separate function
        let image = load(image_reader, ImageFormat::PNG)
            .expect("Image was not a png file!");
        let mut image = image.to_rgba();
        let width = image.width();
        let height = image.height();
        let stride = width * 4;
        let size = stride * height;
        // TODO Split this into its own function
        {
            let pixels = image.enumerate_pixels_mut();
            for (_x, _y, pixel) in pixels {
                let alpha = pixel[3] as u32;
                pixel[0] = weird_math(pixel[0], alpha);
                pixel[1] = weird_math(pixel[1], alpha);
                pixel[2] = weird_math(pixel[2], alpha);

                let tmp = pixel[2];
                pixel[2] = pixel[0];
                pixel[0] = tmp;
            }
        }
        let vec = image.into_vec();
        let mut tmp = tempfile::NamedTempFile::new().expect("Unable to create a tempfile.");
        tmp.set_len(size as u64).expect("Could not truncate length of file");
        tmp.write_all(&*vec).unwrap();
        let pool = shm.create_pool(tmp.as_raw_fd(), size as i32);
        let cursor_buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, WlShmFormat::Argb8888);
        cursor_surface.attach(Some(&cursor_buffer), 0, 0);
        ::std::mem::forget(cursor_buffer);
    }
    return cursor_surface
}

/// Main loop for rendering backgrounds.
/// Need to keep the surface alive, and update it if the
/// user wants to change the background.
#[allow(unused_variables)]
fn main_background_loop(background_surface: WlSurface, cursor_surface: WlSurface, mut event_iter: EventIterator, env: &WaylandEnv) {
    use wayland_client::wayland::WaylandProtocolEvent;
    use wayland_client::Event;
    let seat = env.seat.as_ref().map(|o| &o.0).unwrap();
    let mut pointer = seat.get_pointer();

    pointer.set_event_iterator(&event_iter);
    pointer.set_cursor(0, Some(&cursor_surface), 0, 0);
    background_surface.commit();
    event_iter.sync_roundtrip().unwrap();
    loop {
        for event in &mut event_iter {
            match event {
                Event::Wayland(wayland_event) => {
                    match wayland_event {
                        WaylandProtocolEvent::WlPointer(id, pointer_event) => {
                            match pointer_event {
                                WlPointerEvent::Enter(serial, background_surface, surface_x, surface_y) => {
                                    pointer.set_cursor(0, Some(&cursor_surface), 0, 0);
                                },
                                _ => {
                                }
                            }
                        },
                        _ => {/* unhandled events */}
                    }
                }
                _ => { /* unhandled events */ }
            }
        }
        event_iter.dispatch().expect("Connection with the compositor was lost.");
    }
}


#[test]
fn test_weird_math() {
    assert_eq!(weird_math(10, 254), 9);
    assert_eq!(weird_math(2, 255), 2);
    assert_eq!(weird_math(255, 500), 500);
}

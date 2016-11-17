extern crate rustwlc;
#[macro_use]
extern crate wayland_client;

extern crate tempfile;

extern crate byteorder;
extern crate image;

use std::env;
use std::process::exit;
use std::io::{BufReader};
use image::{load, ImageFormat};
//use gdk_sys;

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() < 2 {
        println!("Please supply either a file path or a color (written in hex)");
        exit(1);
    }
    let input = &args[1];
    // TODO Don't hard code this, get it from the server...
    let output = unsafe {rustwlc::WlcOutput::dummy(0)};
    if let Ok(color) = input.parse::<u32>() {
        let color = Color::from_u32(color);
        generate_solid_background(color, output);
    } else {
        generate_image_background(input.clone(), output);
    }
}

use std::mem::transmute;
use std::os::unix::io::AsRawFd;
use std::io::{Read, Write};
use std::fs::File;
use std::path::PathBuf;

//use gdk_pixbuf::{Pixbuf, InterpType};

use wayland_client::wayland::get_display;
use wayland_client::wayland::compositor::{WlCompositor, WlSurface};
use wayland_client::wayland::shell::{WlShellSurface, WlShell};
use wayland_client::wayland::shm::{WlBuffer, WlShm, WlShmFormat};
use wayland_client::wayland::seat::{WlSeat, WlPointerEvent};
use wayland_client::cursor::load_theme;
use wayland_client::{EventIterator, Proxy};

use rustwlc::WlcOutput;

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

/// Given a solid color, writes bytes associated with that color to
/// a special Wayland surface which is then rendered as a background for Way Cooler.
pub fn generate_solid_background(color: Color, _output: WlcOutput) {
    // Get shortcuts to the globals.
    let (display, iter) = get_display()
        .expect("Unable to connect to a wayland compositor");
    let (env, evt_iter) = WaylandEnv::init(display, iter);
    let compositor = env.compositor.as_ref().map(|o| &o.0).unwrap();
    let shell = env.shell.as_ref().map(|o| &o.0).unwrap();
    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();
    let seat = env.seat.as_ref().map(|o| &o.0).unwrap();

    // Create the surface we are going to write into
    let surface = compositor.create_surface();
    let shell_surface = shell.get_shell_surface(&surface);
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
    surface.attach(Some(&buffer), 0, 0);

    main_background_loop(compositor, shell, shm, seat, surface,
                         shell_surface, buffer, evt_iter);
}

fn weird_math(num: u8, third_num: u32) -> u8 {
    let big_num = num as u32;
    ((big_num * third_num) / 255) as u8
}

#[test]
fn test_weird_math() {
    assert_eq!(weird_math(10, 254), 9);
    assert_eq!(weird_math(2, 255), 2);
    assert_eq!(weird_math(255, 500), 500);
}

/// Given the data from an image, writes it to a special Wayland surface
/// which is then rendered as a background for Way Cooler.
pub fn generate_image_background(path: String, _output: WlcOutput) {
    let image_file = File::open(path.clone())
        .unwrap_or_else(|_| {
            println!("Could not open \"{:?}\"", path);
            panic!("Could not open image file");
        });
    let image_reader = BufReader::new(image_file);
    let image = load(image_reader, ImageFormat::PNG)
        .expect("Image was not a png file!");
    let mut image = image.to_rgba();
    let width = image.width();
    let height = image.height();
    let size = width * height;
    // TODO Split this into its own function
    let mut out_file = File::create("out.bin").unwrap();
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
    out_file.write_all(&*vec).unwrap();
    let tmp = tempfile::NamedTempFile::new().expect("Unable to create a tempfile.");
    tmp.set_len(size as u64).expect("Could not truncate length of file");





/*    //let pix_buf = Pixbuf::new_from_file(path.as_str())
    //    .expect("Could not read the file");
    //let image_file = File::open(path.clone())
    let image_file = ::std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(false)
        .open(path.clone())
        .unwrap_or_else(|_| {
            println!("Could not open \"{:?}\"", path);
            panic!("Could not open image file");
        });
    let raw_fd = image_file.as_raw_fd();
    let map = Mmap::open(&image_file, memmap::Protection::ReadWrite)
        .expect("Could not memory map the temp file");
    let background_surface = ImageSurface::create_from_png(image_file)
        .expect("could not create surface from png");
    // Get shortcuts to the globals.
    let (display, iter) = get_display()
        .expect("Unable to connect to a wayland compositor");
    let (env, evt_iter) = WaylandEnv::init(display, iter);
    let compositor = env.compositor.as_ref().map(|o| &o.0).unwrap();
    let shell = env.shell.as_ref().map(|o| &o.0).unwrap();
    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();
    let seat = env.seat.as_ref().map(|o| &o.0).unwrap();

    // Create the surface we are going to write into
    let surface = compositor.create_surface();
    let shell_surface = shell.get_shell_surface(&surface);
    // Calculate how big the buffer needs to be from the output resolution
    // TODO Get the output size from way cooler somehow...


    let width = background_surface.get_width();
    let height = background_surface.get_height();
    let stride = background_surface.get_stride();
    let size: i32 = stride * height;
    println!("Size: {}, mmap len: {}", size, map.len());
    //let size = map.len() as i32;
    //let tmp = tempfile::NamedTempFile::new().expect("Unable to create a tempfile.");
    //tmp.set_len(size as u64).expect("Could not truncate length of file");
    // Create the buffer that is mem-mapped to the temp file descriptor
    let pool = shm.create_pool(raw_fd, size);
    let buffer = pool.create_buffer(0, width, height, stride, WlShmFormat::Argb8888);
    // Tell Way Cooler not to put this in the tree, treat as background
    // TODO Make this less hacky by actually giving way cooler access to this thing...
    shell_surface.set_class("Background".into());
    // TODO Actually give it the path or something idk
    shell_surface.set_title(format!("Image background yay"));

    // Attach the buffer to the surface
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    surface.set_buffer_scale(1);
    surface.damage(0, 0, width, height);

    println!("{:?}", unsafe{map.as_slice()});
    main_background_loop(compositor, shell, shm, seat, surface,
                         shell_surface, buffer, evt_iter);
    */
}

#[allow(dead_code)]
fn read_image_data(path: PathBuf) -> Vec<u32> {
    let image_file = File::open(path.clone())
        .unwrap_or_else(|_| {
            println!("Could not open \"{:?}\"", path);
            panic!("Could not open image file");
        });
    // Most common screen size
    let mut buffer: Vec<u32> = Vec::with_capacity(1366 * 768);
    // TODO, while clever looking, is Ok(byte) here correct?
    let mut bytes = image_file.bytes();
    'outer: loop {
        let result: u32;
        let mut data_chunk: [u8; 4] = [0; 4];
        for i in 0..3 {
            if let Some(Ok(next_chunk)) = bytes.next() {
                data_chunk[i] = next_chunk;
            } else {
                break 'outer;
            }
        }
        /*let data_chunk: Vec<u8> = next_chunk
            .flat_map(|maybe_bytes| maybe_bytes.into_iter())
            .collect();*/
        result = unsafe {::std::mem::transmute(data_chunk)};
        println!("{:X}", result);
        buffer.push(result);
    }
    buffer
}

/// Main loop for rendering backgrounds.
/// Need to keep the surface alive, and update it if the
/// user wants to change the background.
#[allow(unused_variables)]
fn main_background_loop(compositor: &WlCompositor, shell: &WlShell, shm: &WlShm,
                        seat: &WlSeat,
                        surface: WlSurface, shell_surface: WlShellSurface,
                        buffer: WlBuffer, mut event_iter: EventIterator) {
    use wayland_client::wayland::WaylandProtocolEvent;
    use wayland_client::Event;
    println!("Entering main loop");
    // For now just loop and do nothing
    // Eventually need to query the background state and update
    let cursor_surface = compositor.create_surface();
    let mut pointer = seat.get_pointer();
    let cursor_theme = load_theme(None, 16, shm);
    let maybe_cursor = cursor_theme.get_cursor("default");
    // TODO Uncomment
    /*if maybe_cursor.is_none() {
        println!("Could not load cursor theme properly, cannot load background");
        println!("Please consult the developers about this issue with your distro version, this is a known issue");
        return;
    }
    let cursor = maybe_cursor.unwrap();
    let cursor_buffer = cursor.frame_buffer(0).expect("Couldn't get frame_buffer");
    cursor_surface.attach(Some(&*cursor_buffer), 0, 0);
     */
    pointer.set_event_iterator(&event_iter);
    pointer.set_cursor(0, Some(&cursor_surface), 0, 0);
    surface.commit();
    event_iter.sync_roundtrip().unwrap();
    loop {
        for event in &mut event_iter {
            match event {
                Event::Wayland(wayland_event) => {
                    match wayland_event {
                        WaylandProtocolEvent::WlPointer(id, pointer_event) => {
                            match pointer_event {
                                WlPointerEvent::Enter(serial, surface, surface_x, surface_y) => {
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

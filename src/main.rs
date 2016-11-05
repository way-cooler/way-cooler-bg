extern crate rustwlc;
#[macro_use]
extern crate wayland_client;

extern crate tempfile;

extern crate byteorder;
extern crate gdk_pixbuf;
extern crate cairo;
extern crate gdk_sys;
extern crate memmap;

use std::env;
use std::process::exit;
use cairo::{ImageSurface, Format};
//use gdk_sys;
use memmap::Mmap;

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
pub fn generate_solid_background(color: Color, output: WlcOutput) {
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

/// Given the data from an image, writes it to a special Wayland surface
/// which is then rendered as a background for Way Cooler.
pub fn generate_image_background(path: String, output: WlcOutput) {
    //let pix_buf = Pixbuf::new_from_file(path.as_str())
    //    .expect("Could not read the file");
    let image_file = File::open(path.clone())
    /*let image_file = ::std::fs::OpenOptions::new()
        .write(true)
        .create_new(false)
        .open(path.clone())*/
        .unwrap_or_else(|_| {
            println!("Could not open \"{:?}\"", path);
            panic!("Could not open image file");
        });
    let background_surface = ImageSurface::create_from_png(image_file)
        .expect("could not create surface from png");
    //let cairo_surface_from_pixbuf = gdk_sys::gdk_cairo_surface_create_from_pixbuf();
    //let data = unsafe {pix_buf.get_pixels()};
    //let data = read_image_data(path);
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
    //let resolution = output.get_resolution()
    //    .expect("Couldn't get output resolution");
    //let (width, height) = (resolution.w as i32, resolution.h as i32);
    //let size = (width * height) as i32;
    /*let resolution = output.get_resolution()
        .expect("Couldn't get output resolution");
    let (width, height) = (resolution.w as i32, resolution.h as i32);*/
    //let size = width * height as i32;


    let width = background_surface.get_width();
    let height = background_surface.get_height();
    let stride = background_surface.get_stride();
/*
    let width = 800; let height = 600;
    let stride = width * 4;
*/
    let size: i32 = stride * height;
    let mut tmp = tempfile::NamedTempFile::new().expect("Unable to create a tempfile.");
    tmp.set_len(size as u64).expect("Could not truncate length of file");
    let mut map = Mmap::open(&tmp, memmap::Protection::ReadWrite)
        .expect("Could not memory map the temp file");
    let mem_slice = unsafe { map.as_mut_slice() };
    let boxed_mem: Box<[u8]> = mem_slice.to_vec().into_boxed_slice();

    //let mut index = 0;
    //println!("size of image (bytes): {:?}, rows: {:?}, height: {:?}, skip per row: {:?}",
    //       pix_buf.get_byte_length(), pix_buf.get_width(), pix_buf.get_height(), pix_buf.get_rowstride());
    //for index in 0..size as usize {
    /*let mut counter = 0;
    while index < size as usize {
        if counter >= pix_buf.get_rowstride() / 4 {
            for _ in 0..(width - counter) {
                tmp.write_u32::<NativeEndian>(::std::u32::MAX)
                    .expect("Could not write to temp file to generate image background");
            }
            counter = 0;
        }
        unsafe {
            let bytes = if index < data.len() && index + 3 < data.len(){
                let mut stuff: [u8; 4] = [0; 4];
                // bgra
                // eg, blue, green, red, something else
                stuff[0] = data[index + 2];
                stuff[1] = data[index + 1];
                stuff[2] = data[index];
                stuff[3] = data[index + 3];
                //warn!("stuff: {:#?}", stuff);
                transmute(stuff)
            } else {
                ::std::u32::MAX
            };
            tmp.write_u32::<NativeEndian>(bytes )
                .expect("Could not write to temp file to generate image background");

        }
        index += 4;
        counter += 1;
    }
    println!("pix buf stuff: width: {:?}, stride: {:?}", pix_buf.get_width(), pix_buf.get_rowstride());
    tmp.flush()
        .expect("Could not flush buffer");
    */
    // Create the buffer that is mem-mapped to the temp file descriptor
    let pool = shm.create_pool(tmp.as_raw_fd(), size);
    let buffer = pool.create_buffer(0, width, height, stride, WlShmFormat::Argb8888);
    let cairo_surface = ImageSurface::create_for_data(boxed_mem, free_function, Format::ARgb32,
                                                      width, height, stride);
    let cairo_context = cairo::Context::new(&cairo_surface);
    cairo_context.scale(1.0, 1.0);
    cairo_context.set_source_surface(&background_surface, 0.0, 0.0);
    // TODO Needed??
    cairo_context.paint();
    // Tell Way Cooler not to put this in the tree, treat as background
    shell_surface.set_class("Background".into());
    // TODO Actually give it the path or something idk
    shell_surface.set_title(format!("Image background yay"));
    //let mut string = String::new();
    /*for _ in 0..size {
        unsafe {
            tmp.write_u32::<NativeEndian>(6565656)
                .expect("Could not write to file")
        }
    }*/
    //tmp.read_to_string(&mut string).unwrap();
    //println!("Buffer: {:?}", string);

    // Attach the buffer to the surface
    surface.attach(Some(&buffer), 0, 0);
    surface.set_buffer_scale(1);
    surface.damage(0, 0, width, height);

    main_background_loop(compositor, shell, shm, seat, surface,
                         shell_surface, buffer, evt_iter);
}

fn free_function(_: Box<[u8]>) {
    
}

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

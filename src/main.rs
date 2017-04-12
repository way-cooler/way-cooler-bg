#[macro_use]
extern crate wayland_client;

extern crate tempfile;

extern crate byteorder;
extern crate image;
extern crate dbus;

use std::env;
use std::process::exit;
use image::{GenericImage, DynamicImage, FilterType, load_from_memory, open};
use std::mem::transmute;
use std::os::unix::io::AsRawFd;
use std::io::Write;
use std::str::FromStr;

use wayland_client::wayland::get_display;
use wayland_client::wayland::compositor::{WlCompositor, WlSurface};
use wayland_client::wayland::shell::WlShell;
use wayland_client::wayland::shm::{WlBuffer, WlShm, WlShmFormat};
use wayland_client::wayland::seat::{WlSeat, WlPointerEvent};
use wayland_client::{EventIterator, Proxy};

use byteorder::{NativeEndian, WriteBytesExt};

use dbus::{Connection, Message, MessageItem, BusType};

wayland_env!(WaylandEnv,
             compositor: WlCompositor,
             shell: WlShell,
             shm: WlShm,
             seat: WlSeat
);

const CURSOR: &'static [u8; 656] = include_bytes!("../assets/arrow.png");

// DBus wait time
const DBUS_WAIT_TIME: i32 = 2000;

type BufferResult = Result<WlBuffer, ()>;

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

pub enum BackgroundMode {
    /// Scale image to make the shortest dimension (i.e. height or width)
    /// fit it's container pertaining aspect ratio.
    Fill,

    /// Scale image width to fit container width pertaining aspect ratio.
    Fit,

    /// Scale height and width to fit container's. May create distortion.
    Stretch,

    /// Do not scale image and place to center.
    Center,

    /// Do not scale image and create repeated image forming tile-pattern.
    Tile,
}

impl FromStr for BackgroundMode {
    type Err = String;

    fn from_str(s: &str) -> Result<BackgroundMode, String> {
        match s {
            "fill"    => Ok(BackgroundMode::Fill),
            "fit"     => Ok(BackgroundMode::Fit),
            "stretch" => Ok(BackgroundMode::Stretch),
            "center"  => Ok(BackgroundMode::Center),
            "tile"    => Ok(BackgroundMode::Tile),
            _         => Err(String::from_str(s).unwrap()),
        }
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() < 1 {
        println!("Please supply either a file path or a color (written in hex)");
        exit(1);
    }
    let input = &args[1];

    let (display, iter) = get_display()
        .expect("Unable to connect to a wayland compositor");
    let (env, evt_iter) = WaylandEnv::init(display, iter);
    let compositor = env.compositor.as_ref().map(|o| &o.0).unwrap();
    let shell = env.shell.as_ref().map(|o| &o.0).unwrap();
    let mut background_surface = compositor.create_surface();
    let shell_surface = shell.get_shell_surface(&background_surface);
    shell_surface.set_class("Background".into());
    // TODO Actually give it the path or something idk
    shell_surface.set_title(input.clone());

    // We need to hold on to this buffer, this holds the background image!
    let _background_buffer = if let Ok(color) = input.parse::<u32>() {
        let color = Color::from_u32(color);
        generate_solid_background(color, &mut background_surface, &env)
    } else {
        let mode = BackgroundMode::from_str(if args.len() >= 3 { &args[2] } else { "" });
        generate_image_background(input.as_str(), mode, &mut background_surface, &env)
    }.expect("could not generate image");

    background_surface.commit();
    background_surface.set_buffer_scale(1);
    let mut cursor_surface = compositor.create_surface();
    let _cursor_buffer = self::cursor_surface(&mut cursor_surface, &env);
    main_background_loop(background_surface, cursor_surface, evt_iter, &env);
}

fn rgba_conversion(num: u8, third_num: u32) -> u8 {
    let big_num = num as u32;
    ((big_num * third_num) / 255) as u8
}

fn get_screen_resolution(con: Connection) -> (i32, i32) {
    let screens_msg = Message::new_method_call("org.way-cooler",
                                               "/org/way_cooler/Screen",
                                               "org.way_cooler.Screen",
                                               "List")
        .expect("Could not construct message -- is Way Cooler running?");
    let screen_r = con.send_with_reply_and_block(screens_msg, DBUS_WAIT_TIME)
        .expect("Could not talk to Way Cooler -- is Way Cooler running?");
    let screen_r = &screen_r.get_items()[0];
    let output_id = match screen_r {
        &MessageItem::Array(ref items, _) => {
            match items[0] {
                MessageItem::Str(ref string) => string.clone(),
                _ => panic!("Array didn't contain output id")
            }
        }
        _ => panic!("Wrong type from Screen")
    };
    let res_msg = Message::new_method_call("org.way-cooler",
                                           "/org/way_cooler/Screen",
                                           "org.way_cooler.Screen",
                                           "Resolution")
        .expect("Could not construct message -- is Way Cooler running?")
        .append(MessageItem::Str(output_id));
    let reply: MessageItem = con.send_with_reply_and_block(res_msg, DBUS_WAIT_TIME)
        .expect("Could not talk to Way Cooler -- is Way Cooler running?")
        .get1()
        .expect("Way Cooler returned an unexpected value");
    match reply {
        MessageItem::Struct(items) => {
            let (width, height) = (
                (&items[0]).inner::<u32>()
                    .expect("Way Cooler returned an unexpected value"),
                (&items[1]).inner::<u32>()
                    .expect("Way Cooler returned an unexpected value")
            );
            println!("{:?}, {:?}", width, height);
            (width as i32, height as i32)
        },
        _ => panic!("Could not get resolution of screen")
    }
}

/// Given a solid color, writes bytes associated with that color to
/// a special Wayland surface which is then rendered as a background for Way Cooler.
fn generate_solid_background(color: Color, background_surface: &mut WlSurface,
                                 env: &WaylandEnv) -> BufferResult {
    // Get shortcuts to the globals.
    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();

    // Create the surface we are going to write into
    let mut tmp = tempfile::tempfile().ok().expect("Unable to create a tempfile.");

    // Calculate how big the buffer needs to be from the output resolution
    let dbus_con = Connection::get_private(BusType::Session).unwrap();
    let (width, height) = get_screen_resolution(dbus_con);
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
    let background_buffer = pool.create_buffer(0, width, height, width, WlShmFormat::Argb8888);
    // Tell Way Cooler not to set put this in the tree, treat as background

    // Attach the buffer to the surface
    background_surface.attach(Some(&background_buffer), 0, 0);
    Ok(background_buffer)
}

/// Given the data from an image, writes it to a special Wayland surface
/// which is then rendered as a background for Way Cooler.
fn generate_image_background(path: &str, mode: BackgroundMode, background_surface: &mut WlSurface,
                             env: &WaylandEnv) -> BufferResult {
    // TODO support more formats, split into separate function
    let image = open(path)
        .unwrap_or_else(|_| {
            println!("Could not open image file \"{:?}\"", path);
            ::std::process::exit(1);
        });
    /*let image = load_from_memory(CURSOR)
        .expect("Could not read cursor data, report to maintainer!");*/
    let dbus_con = Connection::get_private(BusType::Session).unwrap();
    let resolution = get_screen_resolution(dbus_con);
    let (scr_width, scr_height) = (resolution.0 as u32, resolution.1 as u32);

    let img_width = image.width();
    let img_height = image.height();

    // Mode image processing
    // The output must be scr_width x scr_height resolution
    let image = match mode {
        BackgroundMode::Fill    => {
            // Find fit scale
            let width_sr: f64  = scr_width as f64 / img_width as f64;
            let height_sr: f64 = scr_height as f64 / img_height as f64;
            let scale_ratio: f64 = if width_sr > height_sr {
                width_sr
            } else {
                height_sr
            };
            let img_width = (scale_ratio * img_width as f64) as u32;
            let img_height = (scale_ratio * img_height as f64) as u32;

            let mut image = image.resize(img_width, img_height, FilterType::Gaussian);
            image.crop(((img_width as i32 - scr_width as i32) / 2).abs() as u32,
                ((img_height as i32 - scr_height as i32) / 2).abs() as u32,
                scr_width,
                scr_height)
        },
        // TODO: Padding background color
        BackgroundMode::Fit     => {
            // Find fit scale ratio
            let width_sr: f64  = scr_width as f64 / img_width as f64;
            let height_sr: f64 = scr_height as f64 / img_height as f64;
            let scale_ratio: f64 = if width_sr < height_sr {
                width_sr
            } else {
                height_sr
            };
            let img_width = (scale_ratio * img_width as f64) as u32;
            let img_height = (scale_ratio * img_height as f64) as u32;

            let image = image.resize(img_width, img_height, FilterType::Gaussian);

            let mut imagepad = DynamicImage::new_rgba8(scr_width, scr_height);
            imagepad.copy_from(&image, 0, ((scr_height - img_height) / 2) as u32);

            imagepad
        },
        BackgroundMode::Stretch => {
            image.resize_exact(scr_width, scr_height, FilterType::Gaussian)
        },
        // TODO: Padding background color
        BackgroundMode::Center  => {
            // FIXME: How if image bigger than screen size?
            let mut imagepad = DynamicImage::new_rgba8(scr_width, scr_height);
            imagepad.copy_from(&image, ((scr_width - img_width) / 2) as u32, ((scr_height - img_height) / 2) as u32);

            imagepad
        },
        BackgroundMode::Tile    => {
            let repeat_x_count: u32 = ((scr_width / img_width) as f64).ceil() as u32 + 1;
            let repeat_y_count: u32 = ((scr_height / img_height) as f64).ceil() as u32 + 1;
            println!("Image w: {}, h: {}", img_width, img_height);
            println!("Repeat x: {}, y: {}", repeat_x_count, repeat_y_count);

            let mut imagepad = DynamicImage::new_rgba8(img_width * repeat_x_count, img_height * repeat_y_count);
            for x in 0..repeat_x_count {
                for y in 0..repeat_y_count {
                    imagepad.copy_from(&image, x * img_width, y * img_height);
                }
            }
            imagepad.crop(0, 0, scr_width, scr_height)
        },
    };

    let img_height = scr_height;
    let img_width = scr_width;
    let img_stride = img_width * 4;
    let img_size = img_stride * img_height;

    let mut image_rgba = image.to_rgba();

    // TODO Split this into its own function
    {
        let pixels = image_rgba.enumerate_pixels_mut();
        for (_x, _y, pixel) in pixels {
            let alpha = pixel[3] as u32;
            pixel[0] = rgba_conversion(pixel[0], alpha);
            pixel[1] = rgba_conversion(pixel[1], alpha);
            pixel[2] = rgba_conversion(pixel[2], alpha);

            let tmp = pixel[2];
            pixel[2] = pixel[0];
            pixel[0] = tmp;
        }
    }

    let vec = image_rgba.into_vec();
    let mut tmp = tempfile::NamedTempFile::new().expect("Unable to create a tempfile.");
    tmp.set_len(img_size as u64).expect("Could not truncate length of file");
    tmp.write_all(&*vec).unwrap();

    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();

    // Create the surface we are going to write into
    let pool = shm.create_pool(tmp.as_raw_fd(), img_size as i32);
    let background_buffer = pool.create_buffer(0, scr_width as i32, scr_height as i32, img_stride as i32, WlShmFormat::Argb8888);

    // Attach the buffer to the surface
    background_surface.attach(Some(&background_buffer), 0, 0);
    background_surface.damage(0, 0, scr_width as i32, scr_height as i32);
    Ok(background_buffer)
}

fn cursor_surface(cursor_surface: &mut WlSurface, env: &WaylandEnv) -> BufferResult {
    let shm = env.shm.as_ref().map(|o| &o.0).unwrap();

    let image = load_from_memory(CURSOR)
        .expect("Could not read cursor data, report to maintainer!");
    let mut image = image.to_rgba();
    let width = image.width();
    let height = image.height();
    let stride = width * 4;
    let size = stride * height;
    {
        let pixels = image.enumerate_pixels_mut();
        for (_x, _y, pixel) in pixels {
            let alpha = pixel[3] as u32;
            pixel[0] = rgba_conversion(pixel[0], alpha);
            pixel[1] = rgba_conversion(pixel[1], alpha);
            pixel[2] = rgba_conversion(pixel[2], alpha);

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
    Ok(cursor_buffer)
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
fn test_rgba_conversion() {
    assert_eq!(rgba_conversion(10, 254), 9);
    assert_eq!(rgba_conversion(2, 255), 2);
    assert_eq!(rgba_conversion(255, 500), 500);
}

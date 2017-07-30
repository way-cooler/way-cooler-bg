#[macro_use]
extern crate wayland_client;

extern crate tempfile;

extern crate byteorder;
extern crate image;
extern crate dbus;
extern crate clap;

mod color;
use color::Color;

use std::mem::transmute;
use std::os::unix::io::AsRawFd;
use std::io::Write;
use std::str::FromStr;
use std::cmp::{min, max};

use wayland_client::wayland::get_display;
use wayland_client::wayland::compositor::{WlCompositor, WlSurface};
use wayland_client::wayland::shell::WlShell;
use wayland_client::wayland::shm::{WlBuffer, WlShm, WlShmFormat};
use wayland_client::wayland::seat::{WlSeat, WlPointerEvent};
use wayland_client::{EventIterator, Proxy};

use byteorder::{NativeEndian, WriteBytesExt};
use clap::{Arg, App};
use image::{GenericImage, DynamicImage, Pixel, FilterType, load_from_memory, open};
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
    let matches = App::new("WayCooler Background Service")
        .version("0.1.0")
        .author("Timidger <APragmaticPlace@gmail.com>")
        .about("Service which manage and provide background to WayCooler window manager.")
        .arg(Arg::with_name("color")
            .short("c")
            .long("color")
            .value_name("HEX")
            .help("Six digit hexa RGB code to render color background e.g. 'ffffff'
                  NOTE: No preceding '#' or '0x'")
            .required_unless("image"))
        .arg(Arg::with_name("image")
            .short("f")
            .long("image")
            .value_name("FILE")
            .help("Path to background image (PNG, JPG, BMP, GIF)")
            .required_unless("color"))
        .arg(Arg::with_name("mode")
            .short("m")
            .long("mode")
            .value_name("BG_MODE")
            .help("Mode affecting image render on screen (fill, fit, stretch, tile)")
            .requires("image"))
        .get_matches();

    let color: Color = if let Some(color) = matches.value_of("color") {
        match color.parse::<u32>() {
            Ok(c) => c.into(),
            Err(_) => {
                let c = u32::from_str_radix(color, 16).unwrap();
                c.into()
            }
        }
    } else {
        let color = u32::from_str_radix("333333", 16).unwrap();
        color.into()
    };

    let (image, mode) = if let Some(image) = matches.value_of("image") {
        let mode = if let Some(mode) = matches.value_of("mode") {
            mode.parse::<BackgroundMode>()
                .expect("Invalid background mode")
        } else {
            BackgroundMode::Fill
        };

        (image.to_string(), mode)
    } else {
        ("".to_string(), BackgroundMode::Fill)
    };

    let (display, iter) = get_display()
        .expect("Unable to connect to a wayland compositor");
    let (env, evt_iter) = WaylandEnv::init(display, iter);
    let compositor = env.compositor.as_ref().map(|o| &o.0).unwrap();
    let shell = env.shell.as_ref().map(|o| &o.0).unwrap();
    let mut background_surface = compositor.create_surface();
    let shell_surface = shell.get_shell_surface(&background_surface);
    shell_surface.set_class("Background".into());

    let _background_buffer = if image.is_empty() {
        shell_surface.set_title(format!("Background Color: {}", color.to_u32()));

        generate_solid_background(color, &mut background_surface, &env)
    } else {
        // TODO Actually give it the path or something idk
        shell_surface.set_title(format!("Background Image: {}", image));

        generate_image_background(image.as_ref(), mode, color, &mut background_surface, &env)
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
            tmp.write_u32::<NativeEndian>(transmute(color.to_u32()))
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

fn fill_image_base_color(image: DynamicImage, color: Color) -> DynamicImage {
    let color = color.to_u8s();
    let color = image::Rgb::from_channels(color.2, color.1, color.0 , color.3);
    let buffer = if let DynamicImage::ImageRgb8(mut buffer) = image {
        for (_, _, p) in buffer.enumerate_pixels_mut() {
            *p = color;
        }
        Ok(buffer)
    } else {
        Err("image has wrong variant")
    }.unwrap();

    DynamicImage::ImageRgb8(buffer)
}

/// Given the data from an image, writes it to a special Wayland surface
/// which is then rendered as a background for Way Cooler.
fn generate_image_background(path: &str, mode: BackgroundMode, color: Color,
                             background_surface: &mut WlSurface, env: &WaylandEnv) -> BufferResult {
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

            let imagepad = DynamicImage::new_rgb8(scr_width, scr_height);
            let mut imagepad = fill_image_base_color(imagepad, color);
            imagepad.copy_from(&image, 0, ((scr_height - img_height) / 2) as u32);

            imagepad
        },
        BackgroundMode::Stretch => {
            image.resize_exact(scr_width, scr_height, FilterType::Gaussian)
        },
        BackgroundMode::Center  => {
            let width_diff: i32 = scr_width as i32 - img_width as i32;
            let height_diff: i32 = scr_height as i32 - img_height as i32;

            let mut image = image;
            let image = image.sub_image(max(-width_diff, 0) as u32 / 2,
                max(-height_diff, 0) as u32 / 2,
                min(scr_width, img_width),
                min(scr_height, img_height));

            let imagepad = DynamicImage::new_rgb8(scr_width, scr_height);
            let mut imagepad = fill_image_base_color(imagepad, color);

            let wpad = max(width_diff, 0) / 2;
            let hpad = max(height_diff, 0) / 2;
            imagepad.copy_from(&image, wpad as u32, hpad as u32);

            imagepad
        },
        BackgroundMode::Tile    => {
            let repeat_x_count: u32 = (scr_width as f64 / img_width as f64).ceil() as u32;
            let repeat_y_count: u32 = (scr_height as f64 / img_height as f64).ceil() as u32;

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

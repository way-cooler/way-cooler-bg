#[macro_use] extern crate wayland_client;
#[macro_use] extern crate wayland_sys;

extern crate tempfile;

extern crate byteorder;
extern crate image;
extern crate clap;
#[macro_use] mod macros;

mod color;
mod resolution;
use resolution::Resolution;
use color::Color;

use std::mem::transmute;
use std::os::unix::io::AsRawFd;
use std::io::Write;
use std::str::FromStr;
use std::cmp::{min, max};

use wayland_client::EnvHandler;
use wayland_client::protocol::{wl_compositor, wl_shell, wl_shell_surface,
                               wl_shm, wl_surface, wl_seat, wl_buffer,
                               wl_output};
use wl_shell_surface::FullscreenMethod;
use wl_output::WlOutput;
use wl_shell::WlShell;
use wl_seat::WlSeat;
use wl_compositor::WlCompositor;

use wl_shm::Format as WlShmFormat;

use byteorder::{NativeEndian, WriteBytesExt};
use clap::{Arg, App};
use image::{GenericImage, DynamicImage, Pixel, FilterType, load_from_memory, open};

wayland_env!(WaylandEnv,
             compositor: wl_compositor::WlCompositor,
             shell: wl_shell::WlShell,
             shm: wl_shm::WlShm,
             seat: wl_seat::WlSeat,
             output: wl_output::WlOutput
);

mod generated {
    // Generated code generally doesn't follow standards
    #![allow(dead_code,non_camel_case_types,unused_unsafe,unused_variables)]
    #![allow(non_upper_case_globals,non_snake_case,unused_imports)]

    pub mod interfaces {
        #[doc(hidden)]
        use wayland_client::protocol_interfaces::{wl_output_interface, wl_surface_interface};
        include!(concat!(env!("OUT_DIR"), "/desktop-shell_interface.rs"));
    }

    pub mod client {
        #[doc(hidden)]
        use wayland_client::{Handler, Liveness, EventQueueHandle, Proxy, RequestResult};
        #[doc(hidden)]
        use wayland_client::protocol::{wl_compositor, wl_shell, wl_shm, wl_surface,
                                       wl_seat, wl_keyboard, wl_buffer,
                                       wl_output, wl_registry};
        use super::interfaces;
        include!(concat!(env!("OUT_DIR"), "/desktop-shell_api.rs"));
    }
}

use generated::client::desktop_shell::DesktopShell;

const CURSOR: &'static [u8; 656] = include_bytes!("../assets/arrow.png");

type BufferResult = Result<wl_buffer::WlBuffer, ()>;

#[derive(Debug, Clone, Copy)]
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
                  NOTE: No preceding '#' or '0x'"))
        .arg(Arg::with_name("image")
            .short("f")
            .long("image")
            .value_name("FILE")
            .help("Path to background image (PNG, JPG, BMP, GIF)"))
        .arg(Arg::with_name("mode")
            .short("m")
            .long("mode")
            .value_name("BG_MODE")
            .help("Mode affecting image render on screen (fill, fit, stretch, tile)")
            .requires("image"))
        .get_matches();
    let default_color = u32::from_str_radix("333333", 16).unwrap().into();
    let (color, mut image) = match (matches.value_of("color"), matches.value_of("image")) {
        (None, None) => (default_color, Some(("".into(), BackgroundMode::Fill))),
        (_, Some(image)) => {
            let mode = matches.value_of("mode")
                .map(|mode| mode.parse::<BackgroundMode>().expect("Invalid background mode"))
                .unwrap_or(BackgroundMode::Fill);
            (default_color, Some((image.to_string(), mode)))
        },
        (Some(color), None) => {
            let color: Color = color.parse::<u32>()
                .map(|c| c.into())
                .unwrap_or_else(|_| {
                    let c = u32::from_str_radix(color, 16).expect("Could not parse color");
                    c.into()
                });
            (color, None)
        }
    };

    let (display, mut event_queue) = wayland_client::default_connect()
        .expect("Unable to connect to a wayland compositor");
    let env_id = event_queue.add_handler(EnvHandler::<WaylandEnv>::new());
    let registry = display.get_registry();
    event_queue.register::<_, EnvHandler<WaylandEnv>>(&registry, env_id);
    // a roundtrip sync will dispatch all event declaring globals to the handler
    // This will make all the globals usable.
    event_queue.sync_roundtrip().expect("Could not sync roundtrip");
    let desktop_shell = match get_wayland!(env_id, &registry, &mut event_queue, DesktopShell, "desktop_shell") {
        Some(shell) => shell,
        None => {
            eprintln!("Please make sure you're running the correct version of Way Cooler");
            eprintln!("This program only supports versions >= 0.7");
            ::std::process::exit(1);
        }
    };
    let outputs = get_all_wayland!(env_id, &registry, &mut event_queue, WlOutput, "wl_output").unwrap();

    let seat = get_wayland!(env_id, &registry, &mut event_queue, WlSeat, "wl_seat").unwrap();
    let pointer = seat.get_pointer().expect("Could not get pointer from seat global");
    let shell = get_wayland!(env_id, &registry, &mut event_queue, WlShell, "wl_shell").unwrap();
    let compositor = get_wayland!(env_id, &registry, &mut event_queue, WlCompositor, "wl_compositor").unwrap();
    let mut cursor_surface = compositor.create_surface();
    let _cursor_buffer = self::cursor_surface(&mut cursor_surface, &mut event_queue, env_id);
    let resolutions: Vec<usize> = outputs.iter()
        .map(|output| {
            let res = Resolution::new();
            let resolution_id = event_queue.add_handler(res);
            event_queue.register::<_, Resolution>(&output, resolution_id);
            resolution_id
        }).collect();
    let mut bg_metadata = outputs.iter().zip(resolutions).map(|(output, res_id)| {
        let background_surface = compositor.create_surface();
        desktop_shell.set_background(output, &background_surface);
        (output, res_id, background_surface)
    });
    event_queue.dispatch()
        .expect("Could not dispatch queue");
    for (output, resolution_id, mut background_surface) in &mut bg_metadata {
        let resolution: Resolution = { *event_queue.state().get_handler(resolution_id) };
        assert!(resolution.w * resolution.h != 0);
        let shell_surface = shell.get_shell_surface(&background_surface);
        shell_surface.set_class("Background".into());
        shell_surface.set_fullscreen(FullscreenMethod::Default, 0, Some(&output));
        shell_surface.set_maximized(Some(&output));
        match image {
            None => {
                shell_surface.set_title(format!("Background Color: {}", color.to_u32()));

                generate_solid_background(color, resolution, &mut event_queue, &mut background_surface, env_id)
            },
            Some((ref mut image, mode)) => {
                if image.is_empty() {
                    shell_surface.set_title("Official background".into());
                } else {
                    shell_surface.set_title(format!("Background Image: {}", image));
                }

                generate_image_background(image.as_ref(),
                                          resolution,
                                          &mut event_queue,
                                          mode,
                                          color,
                                          &mut background_surface,
                                          env_id)
            }
        }.expect("could not generate image");

        background_surface.commit();
        background_surface.set_buffer_scale(1);
    }
    loop {
        display.flush()
            .expect("Could not flush display");
        event_queue.dispatch()
            .expect("Could not dispatch queue");
        pointer.set_cursor(0, Some(&cursor_surface), 0, 0)
            .expect("Could not set cursor");
    }
}

fn rgba_conversion(num: u8, third_num: u32) -> u8 {
    let big_num = num as u32;
    ((big_num * third_num) / 255) as u8
}

/// Given a solid color, writes bytes associated with that color to
/// a special Wayland surface which is then rendered as a background for Way Cooler.
fn generate_solid_background(color: Color,
                             resolution: Resolution,
                             event_queue: &mut wayland_client::EventQueue,
                             background_surface: &mut wl_surface::WlSurface,
                             env_id: usize) -> BufferResult {
    // Get shortcuts to the globals.
    let state = event_queue.state();
    let env = state.get_handler::<EnvHandler<WaylandEnv>>(env_id);
    let shm = &env.shm;

    // Create the surface we are going to write into
    let mut tmp = tempfile::tempfile().ok().expect("Unable to create a tempfile.");

    // Calculate how big the buffer needs to be from the output resolution
    let size = (resolution.w * resolution.h) as i32;

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
    let background_buffer = pool.create_buffer(0,
                                               resolution.w as i32,
                                               resolution.h as i32,
                                               resolution.w as i32,
                                               WlShmFormat::Argb8888)
        .expect("Could not create buffer");
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
fn generate_image_background(path: &str,
                             resolution: Resolution,
                             event_queue: &mut wayland_client::EventQueue,
                             mode: BackgroundMode,
                             color: Color,
                             background_surface: &mut wl_surface::WlSurface,
                             env_id: usize) -> BufferResult {
    // TODO support more formats, split into separate function
    let state = event_queue.state();
    let env = state.get_handler::<EnvHandler<WaylandEnv>>(env_id);
    let image = open(path)
        .unwrap_or_else(|_| {
            load_from_memory(include_bytes!("../assets/official-background.png"))
                .expect("Could not read in official background image")
        });
    let (scr_width, scr_height) = (resolution.w as u32, resolution.h as u32);

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

    let shm = &env.shm;

    // Create the surface we are going to write into
    let pool = shm.create_pool(tmp.as_raw_fd(), img_size as i32);
    let background_buffer = pool.create_buffer(0, scr_width as i32, scr_height as i32, img_stride as i32, WlShmFormat::Argb8888)
        .expect("Could not create buffer");

    // Attach the buffer to the surface
    background_surface.attach(Some(&background_buffer), 0, 0);
    background_surface.damage(0, 0, scr_width as i32, scr_height as i32);
    Ok(background_buffer)
}

fn cursor_surface(cursor_surface: &mut wl_surface::WlSurface,
                  event_queue: &mut wayland_client::EventQueue,
                  env_id: usize) -> BufferResult {
    let state = event_queue.state();
    let env = state.get_handler::<EnvHandler<WaylandEnv>>(env_id);
    let shm = &env.shm;

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
    let cursor_buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, WlShmFormat::Argb8888)
        .expect("Could not create buffer");
    cursor_surface.attach(Some(&cursor_buffer), 0, 0);
    Ok(cursor_buffer)
}


#[test]
fn test_rgba_conversion() {
    assert_eq!(rgba_conversion(10, 254), 9);
    assert_eq!(rgba_conversion(2, 255), 2);
    assert_eq!(rgba_conversion(255, 500), 500);
}

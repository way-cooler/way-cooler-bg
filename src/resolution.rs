use wayland_client::EventQueueHandle;
use wayland_client::protocol::wl_output;

/// Used to know how big to make the surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resolution {
    pub w: u32,
    pub h: u32
}

impl Resolution {
    pub fn new() -> Self {
        Resolution {
            w: 0,
            h: 0
        }
    }
}

impl wl_output::Handler for Resolution {
    fn mode(&mut self,
            _evqh: &mut EventQueueHandle,
            _proxy: &wl_output::WlOutput,
            _flags: wl_output::Mode,
            width: i32,
            height: i32,
            _refresh: i32) {
        self.w = width as u32;
        self.h = height as u32;
    }
}

declare_handler!(Resolution, wl_output::Handler, wl_output::WlOutput);

use super::super::shared_framebuf::PollTimers;
use super::gstreamer_sys::LibGStreamer;
use super::linux_media::CxLinuxMedia;
use super::linux_video_player::LinuxVideoPlayer;

use crate::{cx::Cx, makepad_live_id::LiveId, opengl_cx::OpenglCx, CxOsApi, OpenUrlInPlace};
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Instant};
// Import OpenglCx from x11 for the unified type

/// Scroll distance in logical pixels for one wheel detent, roughly three lines of
/// text, matching the common Windows and GTK defaults.
pub const PIXELS_PER_WHEEL_DETENT: f64 = 60.0;

fn env_var_is_nonempty(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

#[cfg(not(wayland_only))]
fn is_stdin_loop_mode() -> bool {
    crate::app_main::should_run_stdin_loop_from_env()
}

fn forced_windowing_protocol_from_args() -> Option<WindowingProtocol> {
    for arg in std::env::args() {
        if let Some(value) = arg.strip_prefix("--linux-backend=") {
            match value {
                "wayland" => return Some(WindowingProtocol::Wayland),
                #[cfg(not(wayland_only))]
                "x11" => return Some(WindowingProtocol::X11),
                _ => {}
            }
        }
    }
    None
}

// Protocol detection for windowing system
fn detect_windowing_protocol() -> WindowingProtocol {
    #[cfg(wayland_only)]
    {
        WindowingProtocol::Wayland
    }

    #[cfg(not(wayland_only))]
    {
        if let Some(protocol) = forced_windowing_protocol_from_args() {
            return protocol;
        }

        if env_var_is_nonempty("WAYLAND_DISPLAY") {
            return WindowingProtocol::Wayland;
        }

        if env_var_is_nonempty("DISPLAY") {
            return WindowingProtocol::X11;
        }

        WindowingProtocol::X11
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowingProtocol {
    #[cfg(not(wayland_only))]
    X11,
    Wayland,
}

impl Cx {
    pub fn event_loop(cx: Rc<RefCell<Cx>>) {
        let protocol = detect_windowing_protocol();

        // Show environment variables
        match std::env::var("WAYLAND_DISPLAY") {
            Ok(wayland_display) if !wayland_display.is_empty() => {
                println!("WAYLAND_DISPLAY: {}", wayland_display);
            }
            Ok(_) => println!("WAYLAND_DISPLAY: <empty>"),
            Err(_) => println!("WAYLAND_DISPLAY: Not set"),
        }

        match std::env::var("DISPLAY") {
            Ok(x11_display) if !x11_display.is_empty() => {
                println!("DISPLAY: {}", x11_display);
            }
            Ok(_) => println!("DISPLAY: <empty>"),
            Err(_) => println!("DISPLAY: Not set"),
        }

        // Show additional environment info
        if let Ok(session_type) = std::env::var("XDG_SESSION_TYPE") {
            println!("XDG_SESSION_TYPE: {}", session_type);
        }

        if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
            println!("XDG_CURRENT_DESKTOP: {}", desktop);
        }

        // Show the decision
        match protocol {
            WindowingProtocol::Wayland => {
                println!("Selected: Wayland backend");
                #[cfg(wayland_only)]
                println!("Reason: Wayland-only build configuration");
                #[cfg(not(wayland_only))]
                if forced_windowing_protocol_from_args() == Some(WindowingProtocol::Wayland) {
                    println!("Reason: --linux-backend=wayland override");
                } else {
                    println!("Reason: WAYLAND_DISPLAY environment variable is set");
                }
            }
            #[cfg(not(wayland_only))]
            WindowingProtocol::X11 => {
                println!("Selected: X11 backend");
                if is_stdin_loop_mode() {
                    println!("Reason: --stdin-loop mode uses X11 stdin backend");
                } else if forced_windowing_protocol_from_args() == Some(WindowingProtocol::X11) {
                    println!("Reason: --linux-backend=x11 override");
                } else if env_var_is_nonempty("DISPLAY") {
                    println!("Reason: DISPLAY environment variable is set");
                } else {
                    println!("Reason: Default fallback (no display variables set)");
                }
            }
        }

        // Launch appropriate backend
        match protocol {
            WindowingProtocol::Wayland => Self::wayland_event_loop(cx),
            #[cfg(not(wayland_only))]
            WindowingProtocol::X11 => Self::x11_event_loop(cx),
        }
    }

    fn wayland_event_loop(cx: Rc<RefCell<Cx>>) {
        super::wayland::linux_wayland::wayland_event_loop(cx)
    }

    #[cfg(not(wayland_only))]
    fn x11_event_loop(cx: Rc<RefCell<Cx>>) {
        super::x11::linux_x11::x11_event_loop(cx)
    }

    pub(crate) fn handle_networking_events(&mut self) {
        self.dispatch_network_runtime_events();
    }
}

impl CxOsApi for Cx {
    fn init_cx_os(&mut self) {
        self.os.start_time = Some(Instant::now());
        if let Some(item) = std::option_env!("MAKEPAD_PACKAGE_DIR") {
            self.package_root = Some(item.to_string());
        }
        self.native_load_dependencies();
    }

    fn spawn_thread<F>(&mut self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::spawn(f);
    }

    fn seconds_since_app_start(&self) -> f64 {
        Instant::now()
            .duration_since(self.os.start_time.unwrap())
            .as_secs_f64()
    }

    fn open_url(&mut self, _url: &str, _in_place: OpenUrlInPlace) {
        crate::error!("open_url not implemented on this platform");
    }
}

// Unified CxOs that can handle both X11 and Wayland
#[derive(Default)]
pub struct CxOs {
    pub(crate) media: CxLinuxMedia,
    pub(crate) stdin_timers: PollTimers,
    pub(crate) start_time: Option<Instant>,
    pub opengl_cx: Option<OpenglCx>,
    pub(crate) video_players: HashMap<LiveId, LinuxVideoPlayer>,
    pub(crate) gstreamer: Option<LibGStreamer>,
}

impl CxOs {
    pub fn init(&mut self) {
        self.start_time = Some(Instant::now());
    }

    pub(crate) fn gl(&self) -> &super::super::gl_sys::LibGl {
        if let Some(ref cx) = self.opengl_cx {
            &cx.libgl
        } else {
            panic!("No OpenGL context available");
        }
    }
}

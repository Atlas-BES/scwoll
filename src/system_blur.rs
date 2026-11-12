//! Native backdrop blur support.
//!
//! egui cannot sample the desktop behind its window, so the "Frost / blur"
//! slider only simulates frosting. This module asks the compositor to blur
//! what is actually behind the window instead:
//!
//! * Wayland (KWin and other compositors): `ext-background-effect-v1`,
//!   falling back to KDE's `org_kde_kwin_blur`
//! * X11 (KWin/XWayland and compatible compositors): the
//!   `_KDE_NET_WM_BLUR_BEHIND_REGION` window property
//!
//! The compositor owns the blur radius. Scwoll's intensity setting controls
//! how much of its own backdrop is faded out so the real blur can show through
//! (see `paint_backdrop` in `main.rs`).

#[cfg(target_os = "linux")]
use std::ffi::c_void;

#[cfg(target_os = "linux")]
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Tracks whether the compositor blur request is active.
pub struct SystemBlur {
    #[cfg(target_os = "linux")]
    platform: Option<Platform>,
    enabled: bool,
    active: bool,
    attempted: bool,
    status: String,
}

#[cfg(target_os = "linux")]
enum Platform {
    Wayland(wayland_blur::WaylandBlur),
    X11(x11_blur::X11Blur),
}

impl SystemBlur {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            platform: None,
            enabled: false,
            active: false,
            attempted: false,
            status: "Off — using Scwoll's simulated frost.".to_owned(),
        }
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    /// True when the compositor accepted the blur request. Used to decide
    /// whether the painted backdrop may fade out to reveal it.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Keep the compositor request in sync with the theme. Call once per frame.
    pub fn sync(&mut self, want: bool, width: u32, height: u32, frame: &eframe::Frame) {
        if !cfg!(target_os = "linux") {
            let _ = (width, height, frame);
            if want && self.status.is_empty() {
                self.status = "System blur is only available on Linux.".to_owned();
            }
            return;
        }

        #[cfg(target_os = "linux")]
        {
            let toggled_on = want && !self.enabled;
            if want && self.platform.is_none() && (toggled_on || !self.attempted) {
                self.attempted = true;
                self.attach(frame);
            }

            if want != self.enabled {
                self.enabled = want;
                self.apply(want, width, height);
            } else if want && self.enabled {
                // On X11 the blur property contains the window rectangle, so
                // refresh it when the window is resized.
                if matches!(self.platform, Some(Platform::X11(_))) {
                    self.apply(true, width, height);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn attach(&mut self, frame: &eframe::Frame) {
        use raw_window_handle::HasDisplayHandle as _;
        use raw_window_handle::HasWindowHandle as _;

        let window = match frame.window_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                self.status = format!("System blur unavailable: no window handle ({e}).");
                return;
            }
        };
        let display = match frame.display_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                self.status = format!("System blur unavailable: no display handle ({e}).");
                return;
            }
        };

        match (window, display) {
            (
                RawWindowHandle::Wayland(window),
                RawDisplayHandle::Wayland(display),
            ) => {
                let display_ptr = display.display.as_ptr() as *mut c_void;
                let surface_ptr = window.surface.as_ptr() as *mut c_void;
                match unsafe { wayland_blur::WaylandBlur::new(display_ptr, surface_ptr) } {
                    Ok(blur) => {
                        self.status = format!(
                            "Active via {} (Wayland).",
                            blur.backend_name()
                        );
                        self.platform = Some(Platform::Wayland(blur));
                    }
                    Err(e) => {
                        self.status = format!("System blur unavailable: {e}");
                    }
                }
            }
            (RawWindowHandle::Xlib(window), _) => {
                match x11_blur::X11Blur::new(window.window as u32) {
                    Ok(blur) => {
                        self.platform = Some(Platform::X11(blur));
                        self.status =
                            "Active via _KDE_NET_WM_BLUR_BEHIND_REGION (X11).".to_owned();
                    }
                    Err(e) => {
                        self.status = format!("System blur unavailable: {e}");
                    }
                }
            }
            (RawWindowHandle::Xcb(window), _) => {
                match x11_blur::X11Blur::new(window.window.get()) {
                    Ok(blur) => {
                        self.platform = Some(Platform::X11(blur));
                        self.status =
                            "Active via _KDE_NET_WM_BLUR_BEHIND_REGION (X11).".to_owned();
                    }
                    Err(e) => {
                        self.status = format!("System blur unavailable: {e}");
                    }
                }
            }
            _ => {
                self.status = "System blur unavailable: unsupported window backend.".to_owned();
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn apply(&mut self, enabled: bool, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        match &mut self.platform {
            Some(Platform::Wayland(blur)) => {
                if let Err(e) = blur.set_enabled(enabled) {
                    self.active = false;
                    self.status = format!("System blur error: {e}");
                } else if enabled {
                    self.active = true;
                    self.status = format!(
                        "Active via {} (Wayland).",
                        blur.backend_name()
                    );
                } else {
                    self.active = false;
                    self.status = "Off — using Scwoll's simulated frost.".to_owned();
                }
            }
            Some(Platform::X11(blur)) => {
                if let Err(e) = blur.set_enabled(enabled, width, height) {
                    self.active = false;
                    self.status = format!("System blur error: {e}");
                } else if enabled {
                    self.active = true;
                    self.status =
                        "Active via _KDE_NET_WM_BLUR_BEHIND_REGION (X11).".to_owned();
                } else {
                    self.active = false;
                    self.status = "Off — using Scwoll's simulated frost.".to_owned();
                }
            }
            None => {
                self.active = false;
                if enabled && !self.status.starts_with("System blur unavailable") {
                    self.status = "System blur unavailable on this desktop.".to_owned();
                } else if !enabled {
                    self.status = "Off — using Scwoll's simulated frost.".to_owned();
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod wayland_blur {
    use std::ffi::c_void;

    use wayland_client::{
        delegate_noop,
        globals::{registry_queue_init, GlobalList, GlobalListContents},
        protocol::{
            wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry, wl_surface::WlSurface,
        },
        Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    };
    use wayland_protocols::ext::background_effect::v1::client::{
        ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
        ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
    };
    use wayland_protocols_plasma::blur::client::{
        org_kde_kwin_blur::OrgKdeKwinBlur,
        org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
    };

    struct State;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _state: &mut Self,
            _proxy: &wl_registry::WlRegistry,
            _event: wl_registry::Event,
            _data: &GlobalListContents,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
        }
    }
    delegate_noop!(State: ignore WlCompositor);
    delegate_noop!(State: ignore WlRegion);
    delegate_noop!(State: ignore ExtBackgroundEffectManagerV1);
    delegate_noop!(State: ignore ExtBackgroundEffectSurfaceV1);
    delegate_noop!(State: ignore OrgKdeKwinBlurManager);
    delegate_noop!(State: ignore OrgKdeKwinBlur);

    enum Manager {
        Ext(ExtBackgroundEffectManagerV1),
        Kde(OrgKdeKwinBlurManager),
    }

    enum Effect {
        Ext(ExtBackgroundEffectSurfaceV1),
        Kde(OrgKdeKwinBlur),
    }

    pub struct WaylandBlur {
        conn: Connection,
        queue: EventQueue<State>,
        _globals: GlobalList,
        compositor: WlCompositor,
        manager: Manager,
        surface: WlSurface,
        effect: Effect,
        backend: &'static str,
        region_set: bool,
    }

    impl WaylandBlur {
        /// # Safety
        ///
        /// `display_ptr` must point at the live `wl_display` that owns
        /// `surface_ptr`, and both must stay alive for at least as long as the
        /// returned value (which is tied to the app window's lifetime).
        pub unsafe fn new(
            display_ptr: *mut c_void,
            surface_ptr: *mut c_void,
        ) -> Result<Self, String> {
            use wayland_client::backend::{Backend, ObjectId};

            let backend = unsafe { Backend::from_foreign_display(display_ptr as *mut _) };
            let conn = Connection::from_backend(backend);

            let (globals, queue) = registry_queue_init::<State>(&conn)
                .map_err(|e| format!("could not read Wayland globals: {e}"))?;
            let qh = queue.handle();

            let compositor: WlCompositor = globals
                .bind(&qh, 1..=1, ())
                .map_err(|e| format!("no wl_compositor: {e}"))?;

            let surface_id = unsafe {
                ObjectId::from_ptr(WlSurface::interface(), surface_ptr as *mut _)
                    .map_err(|e| format!("invalid wl_surface: {e}"))?
            };
            let surface = WlSurface::from_id(&conn, surface_id)
                .map_err(|e| format!("invalid wl_surface proxy: {e}"))?;

            // Prefer the standard staging protocol; fall back to KDE's older
            // org_kde_kwin_blur for compositors that only ship that one.
            let (manager, effect, backend) = match globals
                .bind::<ExtBackgroundEffectManagerV1, _, _>(&qh, 1..=1, ())
            {
                Ok(manager) => {
                    let effect = manager.get_background_effect(&surface, &qh, ());
                    (
                        Manager::Ext(manager),
                        Effect::Ext(effect),
                        "ext-background-effect-v1",
                    )
                }
                Err(_) => {
                    let manager: OrgKdeKwinBlurManager = globals
                        .bind(&qh, 1..=1, ())
                        .map_err(|_| {
                            "compositor does not expose ext-background-effect-v1 or org_kde_kwin_blur"
                                .to_owned()
                        })?;
                    let effect = manager.create(&surface, &qh, ());
                    (
                        Manager::Kde(manager),
                        Effect::Kde(effect),
                        "org_kde_kwin_blur",
                    )
                }
            };

            Ok(Self {
                conn,
                queue,
                _globals: globals,
                compositor,
                manager,
                surface,
                effect,
                backend,
                region_set: false,
            })
        }

        pub fn backend_name(&self) -> &'static str {
            self.backend
        }

        pub fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
            if enabled == self.region_set {
                return Ok(());
            }
            if enabled {
                let region = self.compositor.create_region(&self.queue.handle(), ());
                // The compositor clips the region to the surface size, so a
                // generous rectangle covers the whole window.
                region.add(0, 0, 16_384, 16_384);
                match &self.effect {
                    Effect::Ext(effect) => effect.set_blur_region(Some(&region)),
                    Effect::Kde(effect) => {
                        effect.set_region(Some(&region));
                        effect.commit();
                    }
                }
                region.destroy();
            } else {
                match &self.effect {
                    Effect::Ext(effect) => effect.set_blur_region(None),
                    Effect::Kde(effect) => {
                        effect.set_region(None);
                        effect.commit();
                    }
                }
            }
            self.conn
                .flush()
                .map_err(|e| format!("could not flush Wayland request: {e}"))?;
            self.region_set = enabled;
            Ok(())
        }
    }

    impl Drop for WaylandBlur {
        fn drop(&mut self) {
            match &self.manager {
                Manager::Ext(manager) => manager.destroy(),
                Manager::Kde(manager) => manager.unset(&self.surface),
            }
            match &self.effect {
                Effect::Ext(effect) => effect.destroy(),
                Effect::Kde(effect) => effect.release(),
            }
            let _ = self.conn.flush();
        }
    }
}

#[cfg(target_os = "linux")]
mod x11_blur {
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode},
        rust_connection::RustConnection,
        wrapper::ConnectionExt as _,
    };

    const BLUR_ATOM: &[u8] = b"_KDE_NET_WM_BLUR_BEHIND_REGION";

    pub struct X11Blur {
        conn: RustConnection,
        window: u32,
        atom: u32,
        area: (u32, u32),
        enabled: bool,
    }

    impl X11Blur {
        pub fn new(window: u32) -> Result<Self, String> {
            let (conn, _screen) =
                x11rb::connect(None).map_err(|e| format!("cannot connect to X11: {e}"))?;
            let atom = conn
                .intern_atom(false, BLUR_ATOM)
                .map_err(|e| format!("cannot create blur atom: {e}"))?
                .reply()
                .map_err(|e| format!("cannot read blur atom: {e}"))?
                .atom;
            Ok(Self {
                conn,
                window,
                atom,
                area: (0, 0),
                enabled: false,
            })
        }

        pub fn set_enabled(
            &mut self,
            enabled: bool,
            width: u32,
            height: u32,
        ) -> Result<(), String> {
            let area = (width.max(1), height.max(1));
            let mut dirty = false;
            if enabled {
                if !self.enabled || self.area != area {
                    let data = [0u32, 0, area.0, area.1];
                    self.conn
                        .change_property32(
                            PropMode::REPLACE,
                            self.window,
                            self.atom,
                            AtomEnum::CARDINAL,
                            &data,
                        )
                        .map_err(|e| format!("cannot set blur region: {e}"))?;
                    self.area = area;
                    dirty = true;
                }
            } else if self.enabled {
                self.conn
                    .delete_property(self.window, self.atom)
                    .map_err(|e| format!("cannot clear blur region: {e}"))?;
                dirty = true;
            }
            if dirty {
                self.conn
                    .flush()
                    .map_err(|e| format!("cannot flush X11 request: {e}"))?;
            }
            self.enabled = enabled;
            Ok(())
        }
    }
}


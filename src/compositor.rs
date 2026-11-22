use windows::core::{Interface, Result};
use windows::Foundation::Numerics::{Vector2, Vector3};
use windows::System::DispatcherQueueController;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionBrush, CompositionClip, CompositionRoundedRectangleGeometry, CompositionStretch,
    Compositor, ContainerVisual, SpriteVisual,
};
use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISwapChain1};
use windows::Win32::System::WinRT::Composition::{
    ICompositorDesktopInterop, ICompositorInterop,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, RoInitialize, DQTAT_COM_NONE,
    DQTYPE_THREAD_CURRENT, RO_INIT_SINGLETHREADED,
};

struct DCompBackend {
    device: IDCompositionDevice,
    target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

impl Drop for DCompBackend {
    fn drop(&mut self) {
        unsafe {
            let _ = self.target.SetRoot(None);
            let _ = self.device.Commit();
        }
    }
}

struct WinRtBackend {
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    backdrop_visual: SpriteVisual,
    surface_visual: SpriteVisual,
    backdrop_clip_geometry: CompositionRoundedRectangleGeometry,
}

/// Hosts the composition backend bound to the stationary canvas HWND.
///   directly through `DirectComposition` (`DCompositionCreateDevice` -> `IDCompositionTarget` ->
pub struct CompositorHost {
    hwnd: HWND,
    swap_chain: IDXGISwapChain1,
    _dispatcher_queue: DispatcherQueueController,
    dcomp: Option<DCompBackend>,
    winrt: Option<WinRtBackend>,
    backdrop_visible: bool,
    last_w_px: f32,
    last_h_px: f32,
    last_radius_px: f32,
    last_ear_r_px: f32,
    last_max_w_px: f32,
    last_max_h_px: f32,
}

impl CompositorHost {
    /// Initializes WinRT on the current UI thread as STA (safe to call once at startup).
    pub fn init_sta() -> Result<()> {
        let res = unsafe { RoInitialize(RO_INIT_SINGLETHREADED) };
        match res {
            Ok(()) => Ok(()),
            Err(e) if e.code() == RPC_E_CHANGED_MODE => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Creates the `DispatcherQueueController` and attaches the opaque `DirectComposition`
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hwnd: HWND,
        swap_chain: &IDXGISwapChain1,
        cur_w_px: f32,
        cur_h_px: f32,
        radius_px: f32,
        ear_r_px: f32,
        max_w_px: f32,
        max_h_px: f32,
    ) -> Result<Self> {
        let options = DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        };
        let dispatcher_queue = unsafe { CreateDispatcherQueueController(options)? };
        let dcomp = attach_dcomp(hwnd, swap_chain)?;

        let mut host = Self {
            hwnd,
            swap_chain: swap_chain.clone(),
            _dispatcher_queue: dispatcher_queue,
            dcomp: Some(dcomp),
            winrt: None,
            backdrop_visible: false,
            last_w_px: -1.0,
            last_h_px: -1.0,
            last_radius_px: -1.0,
            last_ear_r_px: -1.0,
            last_max_w_px: -1.0,
            last_max_h_px: -1.0,
        };
        host.update_layout(cur_w_px, cur_h_px, radius_px, ear_r_px, max_w_px, max_h_px)?;
        Ok(host)
    }

    /// Switches between true-black `DirectComposition` (`visible == false`) and translucent
    /// WinRT `HostBackdropBrush` composition (`visible == true`).
    pub fn set_backdrop_visible(&mut self, visible: bool) {
        if self.backdrop_visible == visible {
            return;
        }
        self.backdrop_visible = visible;
        if visible {
            // Detach DirectComposition target first so HWND owns only one composition target.
            self.dcomp = None;
            if let Ok(winrt) = attach_winrt(self.hwnd, &self.swap_chain) {
                self.winrt = Some(winrt);
                self.last_w_px = -1.0;
                self.last_max_w_px = -1.0;
            } else if let Ok(dcomp) = attach_dcomp(self.hwnd, &self.swap_chain) {
                self.dcomp = Some(dcomp);
                self.backdrop_visible = false;
            }
        } else {
            self.winrt = None;
            if let Ok(dcomp) = attach_dcomp(self.hwnd, &self.swap_chain) {
                self.dcomp = Some(dcomp);
            }
        }
    }

    /// Updates composition visual geometry (`backdrop_visual` when translucent WinRT mode is active).
    pub fn update_layout(
        &mut self,
        cur_w_px: f32,
        cur_h_px: f32,
        radius_px: f32,
        ear_r_px: f32,
        max_w_px: f32,
        max_h_px: f32,
    ) -> Result<()> {
        let cur_w = cur_w_px.max(1.0);
        let cur_h = cur_h_px.max(1.0);
        let max_w = max_w_px.max(cur_w);
        let max_h = max_h_px.max(cur_h);
        let ear_r = ear_r_px.clamp(0.0, cur_w * 0.25);
        let body_w = (cur_w - ear_r * 2.0).max(1.0);
        let radius = radius_px.clamp(0.0, (body_w.min(cur_h)) * 0.5);

        let Some(winrt) = &self.winrt else {
            self.last_w_px = cur_w;
            self.last_h_px = cur_h;
            self.last_radius_px = radius;
            self.last_ear_r_px = ear_r;
            self.last_max_w_px = max_w;
            self.last_max_h_px = max_h;
            return Ok(());
        };

        let surface_changed = (max_w - self.last_max_w_px).abs() > 0.05
            || (max_h - self.last_max_h_px).abs() > 0.05;

        if surface_changed {
            let canvas_size = Vector2 { X: max_w, Y: max_h };
            winrt.root.SetSize(canvas_size)?;
            winrt.surface_visual.SetSize(canvas_size)?;
            self.last_max_w_px = max_w;
            self.last_max_h_px = max_h;
        }

        if self.backdrop_visible {
            let pill_changed = surface_changed
                || (cur_w - self.last_w_px).abs() > 0.05
                || (cur_h - self.last_h_px).abs() > 0.05
                || (radius - self.last_radius_px).abs() > 0.05
                || (ear_r - self.last_ear_r_px).abs() > 0.05;

            if pill_changed {
                let offset_x = ((max_w - cur_w) * 0.5) + ear_r;
                winrt.backdrop_visual.SetOffset(Vector3 {
                    X: offset_x,
                    Y: 0.0,
                    Z: 0.0,
                })?;
                winrt.backdrop_visual
                    .SetSize(Vector2 { X: body_w, Y: cur_h })?;
                winrt.backdrop_clip_geometry
                    .SetOffset(Vector2 { X: 0.0, Y: -radius })?;
                winrt.backdrop_clip_geometry.SetSize(Vector2 {
                    X: body_w,
                    Y: cur_h + radius,
                })?;
                winrt.backdrop_clip_geometry
                    .SetCornerRadius(Vector2 { X: radius, Y: radius })?;

                self.last_w_px = cur_w;
                self.last_h_px = cur_h;
                self.last_radius_px = radius;
                self.last_ear_r_px = ear_r;
            }
        }

        Ok(())
    }
}

fn attach_dcomp(hwnd: HWND, swap_chain: &IDXGISwapChain1) -> Result<DCompBackend> {
    let dxgi_device: IDXGIDevice = unsafe { swap_chain.GetDevice()? };
    let device: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi_device)? };
    let target = unsafe { device.CreateTargetForHwnd(hwnd, true)? };
    let visual = unsafe { device.CreateVisual()? };
    unsafe {
        visual.SetContent(swap_chain)?;
        target.SetRoot(&visual)?;
        device.Commit()?;
    }
    Ok(DCompBackend {
        device,
        target,
        _visual: visual,
    })
}

fn attach_winrt(hwnd: HWND, swap_chain: &IDXGISwapChain1) -> Result<WinRtBackend> {
    let compositor = Compositor::new()?;
    let desktop_interop: ICompositorDesktopInterop = compositor.cast()?;
    let target = unsafe { desktop_interop.CreateDesktopWindowTarget(hwnd, false)? };

    let root = compositor.CreateContainerVisual()?;
    target.SetRoot(&root)?;

    let backdrop_clip_geometry = compositor.CreateRoundedRectangleGeometry()?;
    let backdrop_clip: CompositionClip = compositor
        .CreateGeometricClipWithGeometry(&backdrop_clip_geometry)?
        .cast()?;

    let backdrop_visual = compositor.CreateSpriteVisual()?;
    backdrop_visual.SetClip(&backdrop_clip)?;
    if let Ok(host_backdrop) = compositor.CreateHostBackdropBrush() {
        if let Ok(brush) = host_backdrop.cast::<CompositionBrush>() {
            let _ = backdrop_visual.SetBrush(&brush);
        }
    }

    let surface_visual = compositor.CreateSpriteVisual()?;
    surface_visual.SetOffset(Vector3 {
        X: 0.0,
        Y: 0.0,
        Z: 0.0,
    })?;
    bind_swap_chain(&compositor, &surface_visual, swap_chain)?;

    let children = root.Children()?;
    children.InsertAtBottom(&backdrop_visual)?;
    children.InsertAtTop(&surface_visual)?;

    Ok(WinRtBackend {
        _target: target,
        root,
        backdrop_visual,
        surface_visual,
        backdrop_clip_geometry,
    })
}

fn bind_swap_chain(
    compositor: &Compositor,
    surface_visual: &SpriteVisual,
    swap_chain: &IDXGISwapChain1,
) -> Result<()> {
    let comp_interop: ICompositorInterop = compositor.cast()?;
    let comp_surface = unsafe { comp_interop.CreateCompositionSurfaceForSwapChain(swap_chain)? };
    let surface_brush = compositor.CreateSurfaceBrushWithSurface(&comp_surface)?;
    surface_brush.SetStretch(CompositionStretch::None)?;
    let brush: CompositionBrush = surface_brush.cast()?;
    surface_visual.SetBrush(&brush)?;
    Ok(())
}

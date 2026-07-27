use super::backend::DisplayBackend;
use super::display::Framebuffer;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{WindowId, Window, WindowAttributes};
use softbuffer::{Context, Surface};
use std::rc::Rc;

pub struct WinitDisplayBackend {
    width: u32,
    height: u32,
    initialized: bool,
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    framebuffer: Option<Framebuffer>,
}

impl WinitDisplayBackend {
    pub fn new() -> Self {
        Self {
            width: 768,
            height: 768,
            initialized: false,
            window: None,
            context: None,
            surface: None,
            framebuffer: None,
        }
    }
}

impl DisplayBackend for WinitDisplayBackend {
    fn initialize(&mut self) -> Result<(), String> {
        self.initialized = true;
        Ok(())
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn clear(&mut self, color: u32) {
        let mut buffer = self
            .surface
            .as_mut()
            .unwrap()
            .buffer_mut()
            .unwrap();

        for pixel in buffer.iter_mut() {
            *pixel = color;
        }

        buffer.present().unwrap();
    }

    fn mainloop(&mut self) {
        // Do smth to start the app, maybe
        let event_loop = EventLoop::new().unwrap();
        event_loop.run_app(self).unwrap();
    }
    fn present(&mut self, framebuffer: &Framebuffer) {
        let mut buffer = self
            .surface
            .as_mut()
            .unwrap()
            .buffer_mut()
            .unwrap();

        for (src, dst) in framebuffer.pixels.iter().zip(buffer.iter_mut()) {
            *dst = *src
        }
    }
}

impl ApplicationHandler for WinitDisplayBackend {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {}

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default()
                .with_title("Dynamix")
                .with_resizable(false)
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.width as f64,
                    self.height as f64,
                )))
            .expect("Failed to create window");

        let window = Rc::new(window);
        self.window = Some(window.clone());

        let context = Context::new(window.clone()).expect("Failed to create softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("Failed to create surface");
        let framebuffer = Framebuffer::new(self.width, self.height);

        self.framebuffer = Some(framebuffer);
        self.context = Some(context);
        self.surface = Some(surface);

        self.clear(0x7F5AF0);

        println!("Window created!");
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {}

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("Closing Dynamix");
                event_loop.exit();
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, _event: DeviceEvent) {}

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {}

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {}

    fn memory_warning(&mut self, _event_loop: &ActiveEventLoop) {}
}
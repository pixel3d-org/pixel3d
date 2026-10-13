//! pixel3d desktop application.
//!
//! Usage: `pixel3d [--sample] [file.p3d] [models...]`. The window is an eframe app
//! around `p3d_ui::PixelApp` (project spec 20.2): integrated title bar on
//! macOS, the viewport on eframe's wgpu device, preferences in eframe's
//! storage.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

#[cfg(target_os = "macos")]
mod native_menu;

use p3d_ui::{PixelApp, Services};

const USAGE: &str = "\
usage: pixel3d [options] [file.p3d] [model.glb|.gltf|.obj|.fbx]...

options:
      --sample             open the sample scene
      --screenshot <png>   save a screenshot of the window, then quit
      --light              use the light theme for this run
      --control <port>     let agents and tests drive the app (127.0.0.1, 0 picks a port)
      --fresh              start with default preferences and do not save them
      --command <line>     run a command after start (repeatable), e.g. 'ui.mode mode=face'
      --hover <x,y>        with --screenshot: rest the pointer there (points) first
      --click              with --hover: click there once before the screenshot
  -V, --version            print the version
  -h, --help               print this help
";

/// Command-line choices.
#[derive(Default)]
struct Args {
    sample: bool,
    files: Vec<PathBuf>,
    screenshot: Option<PathBuf>,
    light: bool,
    commands: Vec<String>,
    hover: Option<egui::Pos2>,
    click: bool,
    control: Option<u16>,
    fresh: bool,
}

struct App {
    app: PixelApp,
    title: String,
    /// Keep the preferences on quit (not for `--fresh` or screenshot runs).
    save_prefs: bool,
    /// Screenshot runs: where the pointer rests, whether it clicks there
    /// and the frames seen so far.
    hover: Option<egui::Pos2>,
    click: bool,
    frames: u32,
    #[cfg(target_os = "macos")]
    native_menu: Option<native_menu::NativeMenu>,
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            if self.native_menu.is_none() && std::env::var_os("P3D_NO_NATIVE_MENU").is_none() {
                self.native_menu = Some(native_menu::NativeMenu::install());
            }
            if let Some(m) = &self.native_menu {
                m.poll(&mut self.app, ctx);
            }
        }
        self.app.logic(ctx);
        if std::mem::take(&mut self.app.quit_requested) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let title = format!("pixel3d - {}", self.app.title());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }

    /// Keys the app takes before egui (Tab toggles Edit Mode). Screenshot
    /// runs ignore the user's mouse and keyboard so the picture shows the
    /// requested state.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.filter_input(ctx, raw);
        if self.app.quit_after_shot {
            raw.events.retain(|e| matches!(e, egui::Event::Screenshot { .. }));
            if let Some(at) = self.hover {
                // Move there once, then rest (tooltips wait for a still pointer).
                self.frames += 1;
                if self.frames <= 2 {
                    raw.events.push(egui::Event::PointerMoved(at));
                }
                // Press on the third frame, release on the fourth.
                if self.click && (self.frames == 3 || self.frames == 4) {
                    let pressed = self.frames == 3;
                    raw.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Screenshot runs use defaults and must not change the user's preferences.
        if self.save_prefs && !self.app.quit_after_shot {
            storage.set_string(p3d_ui::PREFS_KEY, self.app.prefs_json());
        }
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|| rfd::FileDialog::new().add_filter("pixel3d scene", &["p3d"]).pick_file())),
        pick_save: Some(Box::new(|name: &str| rfd::FileDialog::new().add_filter("pixel3d scene", &["p3d"]).set_file_name(name).save_file())),
        pick_import: Some(Box::new(|| {
            rfd::FileDialog::new().set_title("Import").add_filter("3D models (glTF, GLB, OBJ, FBX)", &p3d_ui::MODEL_EXTENSIONS).pick_file()
        })),
    }
}

/// How frames are presented: vsync by default; `P3D_VSYNC=0` presents
/// without a queue (lowest latency), `P3D_VSYNC=1` forces vsync.
fn surface_config() -> eframe::egui_wgpu::SurfaceConfig {
    let no_vsync = matches!(std::env::var("P3D_VSYNC").ok().as_deref().map(str::trim), Some("0" | "off" | "false"));
    eframe::egui_wgpu::SurfaceConfig {
        present_mode: if no_vsync { eframe::wgpu::PresentMode::AutoNoVsync } else { eframe::wgpu::PresentMode::AutoVsync },
        desired_maximum_frame_latency: Some(1),
    }
}

/// Windows: DirectX 12 with the FXC compiler before wgpu initializes any
/// driver (cadcraft found the GL backend crashing on some AMD drivers).
/// `WGPU_BACKEND` and `WGPU_DX12_COMPILER` still take precedence.
#[cfg(target_os = "windows")]
fn configure_windows(options: &mut eframe::NativeOptions) {
    use eframe::wgpu::{Backends, Dx12Compiler};
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = Backends::from_env().unwrap_or(Backends::DX12);
        create.instance_descriptor.backend_options.dx12.shader_compiler = Dx12Compiler::from_env().unwrap_or(Dx12Compiler::Fxc);
    }
}

fn main() -> ExitCode {
    let level = if std::env::var_os("P3D_LOG").is_some() { tracing::Level::INFO } else { tracing::Level::WARN };
    tracing_subscriber::fmt().with_max_level(level).with_writer(std::io::stderr).init();
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-V" | "--version" => {
                println!("pixel3d {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--sample" => args.sample = true,
            "--light" => args.light = true,
            "--command" => match it.next() {
                Some(c) => args.commands.push(c),
                None => {
                    eprintln!("error: --command needs a command id\n\n{USAGE}");
                    return ExitCode::FAILURE;
                }
            },
            "--click" => args.click = true,
            "--fresh" => args.fresh = true,
            "--control" => match it.next().and_then(|s| s.parse().ok()) {
                Some(p) => args.control = Some(p),
                None => {
                    eprintln!("error: --control needs a port number\n\n{USAGE}");
                    return ExitCode::FAILURE;
                }
            },
            "--hover" => match it.next().and_then(|s| parse_point(&s)) {
                Some(p) => args.hover = Some(p),
                None => {
                    eprintln!("error: --hover needs a position like 120,80\n\n{USAGE}");
                    return ExitCode::FAILURE;
                }
            },
            "--screenshot" => match it.next() {
                Some(p) => args.screenshot = Some(PathBuf::from(p)),
                None => {
                    eprintln!("error: --screenshot needs a file name\n\n{USAGE}");
                    return ExitCode::FAILURE;
                }
            },
            other if other.starts_with('-') => {
                eprintln!("error: unknown option `{other}`\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
            path => args.files.push(PathBuf::from(path)),
        }
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `x,y` in points.
fn parse_point(s: &str) -> Option<egui::Pos2> {
    let (x, y) = s.split_once(',')?;
    Some(egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn run(args: Args) -> anyhow::Result<()> {
    let mut options = eframe::NativeOptions {
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration::default().with_surface_config(surface_config()),
        viewport: egui::ViewportBuilder::default()
            .with_title("pixel3d")
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_app_id("dev.pixel3d.pixel3d"),
        persist_window: false,
        multisampling: 0,
        depth_buffer: 0,
        ..Default::default()
    };
    #[cfg(target_os = "windows")]
    configure_windows(&mut options);
    // Large scenes need large vertex buffers: ask for what the adapter supports.
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        let base = std::sync::Arc::clone(&create.device_descriptor);
        create.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut d = base(adapter);
            d.required_limits.max_buffer_size = d.required_limits.max_buffer_size.max(adapter.limits().max_buffer_size);
            d
        });
    }
    eframe::run_native(
        "pixel3d",
        options,
        Box::new(move |cc| {
            let mut app = PixelApp::new(p3d_engine::Session::new(), services());
            if args.screenshot.is_none()
                && !args.fresh
                && let Some(prefs) = cc.storage.and_then(|s| s.get_string(p3d_ui::PREFS_KEY))
            {
                app.load_prefs(&prefs);
            }
            app.integrated_titlebar = cfg!(target_os = "macos");
            if let Some(rs) = &cc.wgpu_render_state {
                app.set_wgpu(rs);
            }
            // As if dropped on the window: a scene opens, models are imported.
            app.drop_files(&args.files);
            if args.sample {
                let _ = app.run("file.sample", serde_json::Value::Null);
            }
            // Command lines as the Console takes them: `ui.mode mode=face`.
            for c in &args.commands {
                let outcome = p3d_ui::commands::parse_line(c).and_then(|(id, params)| app.run(&id, params));
                if let Err(e) = outcome {
                    tracing::warn!(command = %c, error = %e, "start-up command failed");
                }
            }
            if args.light {
                app.ui.theme = p3d_ui::theme::ThemePref::Light;
            }
            if let Some(port) = args.control {
                match app.start_control(port) {
                    Ok(addr) => eprintln!("control server: {addr}"),
                    Err(e) => eprintln!("error: control server: {e}"),
                }
            }
            if let Some(path) = &args.screenshot {
                app.shots.request(path.clone());
                app.quit_after_shot = true;
                if args.hover.is_some() {
                    // Long enough for the tooltip delay.
                    app.shots.extra_frames = 60;
                }
            }
            Ok(Box::new(App {
                app,
                title: String::new(),
                save_prefs: !args.fresh,
                hover: args.hover.filter(|_| args.screenshot.is_some()),
                click: args.click,
                frames: 0,
                #[cfg(target_os = "macos")]
                native_menu: None,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

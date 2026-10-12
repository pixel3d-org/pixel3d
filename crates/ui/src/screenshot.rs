//! Window screenshots as PNG files (automation, `--screenshot`, the
//! control server's `ui.screenshot`). A few frames are drawn first so the
//! layout and the viewport image have settled.

use std::path::PathBuf;

/// Frames drawn before the screenshot is taken.
const SETTLE_FRAMES: u32 = 4;

/// A requested screenshot.
#[derive(Debug, Default)]
pub struct Shots {
    pending: Option<(PathBuf, u32, u64)>,
    issued: Option<(PathBuf, u64)>,
    next: u64,
    /// The last finished screenshot: its path or the error.
    pub done: Option<Result<PathBuf, String>>,
    /// Extra frames to wait, for example until a hover tooltip shows.
    pub extra_frames: u32,
}

impl Shots {
    /// Asks for a screenshot saved to `path`; returns its token.
    pub fn request(&mut self, path: PathBuf) -> u64 {
        self.next += 1;
        self.pending = Some((path, 0, self.next));
        self.done = None;
        self.next
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.issued.is_some()
    }

    /// Issues a pending request once the frames have settled, and saves a
    /// screenshot that arrived. Call once per frame.
    ///
    /// Complexity: O(pixels) when a screenshot arrives, O(1) otherwise.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if let Some((path, frames, token)) = self.pending.take() {
            if frames >= SETTLE_FRAMES + self.extra_frames {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(token)));
                self.issued = Some((path, token));
            } else {
                self.pending = Some((path, frames + 1, token));
            }
            ctx.request_repaint();
        }
        let Some((_, token)) = &self.issued else { return };
        let token = *token;
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. } if user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()) == Some(&token) => {
                    Some(image.clone())
                }
                _ => None,
            })
        });
        ctx.request_repaint();
        if let Some(image) = image
            && let Some((path, _)) = self.issued.take()
        {
            self.done = Some(save(&image, &path).map(|()| path));
        }
    }
}

/// Writes an egui screenshot as PNG.
///
/// Complexity: O(pixels).
pub fn save(image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let [w, h] = image.size;
    let mut pixels = Vec::with_capacity(w * h * 4);
    for c in &image.pixels {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        pixels.extend_from_slice(&[r, g, b, a]);
    }
    let img = p3d_render::Image { width: w as u32, height: h as u32, pixels };
    let png = img.to_png().map_err(|e| e.to_string())?;
    std::fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))
}

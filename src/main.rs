mod app;
mod data;
mod model;

use app::PlannerApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("FOUNDRY Plan")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1040.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "FOUNDRY Plan",
        options,
        Box::new(|cc| Ok(Box::new(PlannerApp::new(cc)))),
    )
}

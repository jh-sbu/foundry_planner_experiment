use std::{collections::HashMap, path::PathBuf};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Key, Margin, PointerButton, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, TextStyle, Vec2,
    text::{LayoutJob, TextFormat},
};

use crate::{
    data::{GameData, MachineKind, Recipe, RecipeKind, TEMPLATE_ROOT_ENV, resolve_template_root},
    model::{
        ConnectionState, MachineSettings, NodeId, Plan, PlanEvaluation, PortRef, PortSide,
        blast_furnace_hot_air_rate, blast_furnace_operating_speed, primary_rate_for_machine_count,
        recipe_rate_anchor,
    },
};

const SIDEBAR_WIDTH: f32 = 292.0;
const SUMMARY_WIDTH: f32 = 290.0;
const NODE_WIDTH: f32 = 300.0;
const NODE_HEADER: f32 = 64.0;
const PORT_ROW: f32 = 43.0;
const NODE_FOOTER: f32 = 50.0;
const PORT_RADIUS: f32 = 7.0;

const BG: Color32 = Color32::from_rgb(10, 15, 20);
const PANEL: Color32 = Color32::from_rgb(17, 25, 32);
const CARD: Color32 = Color32::from_rgb(25, 36, 45);
const CARD_ALT: Color32 = Color32::from_rgb(31, 45, 55);
const TEXT: Color32 = Color32::from_rgb(225, 234, 239);
const MUTED: Color32 = Color32::from_rgb(135, 155, 166);
const ORANGE: Color32 = Color32::from_rgb(241, 144, 48);
const CYAN: Color32 = Color32::from_rgb(63, 197, 209);
const RED: Color32 = Color32::from_rgb(239, 83, 93);
const GREEN: Color32 = Color32::from_rgb(73, 205, 132);

#[derive(Clone)]
struct RecipeChooser {
    origin: PortRef,
    screen_position: Pos2,
    world_position: Pos2,
    search: String,
}

pub struct PlannerApp {
    data: GameData,
    data_root: Option<PathBuf>,
    load_message: String,
    recipe_search: String,
    plan: Plan,
    selected_node: Option<NodeId>,
    pan: Vec2,
    zoom: f32,
    dragged_port: Option<PortRef>,
    chooser: Option<RecipeChooser>,
    count_edits: HashMap<NodeId, String>,
    output_edits: HashMap<NodeId, String>,
    next_spawn: usize,
    fit_requested: bool,
}

fn load_game_data() -> (Option<PathBuf>, Result<GameData, String>) {
    match resolve_template_root() {
        Ok(root) => {
            let result = GameData::load(&root);
            (Some(root), result)
        }
        Err(error) => (None, Err(error)),
    }
}

impl PlannerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let (data_root, result) = load_game_data();
        let (data, load_message) = match result {
            Ok(data) => {
                let message = format!(
                    "Loaded {} recipes and {} machines",
                    data.recipes.len(),
                    data.machines.len()
                );
                (data, message)
            }
            Err(error) => (GameData::default(), error),
        };
        Self {
            data,
            data_root,
            load_message,
            recipe_search: String::new(),
            plan: Plan::default(),
            selected_node: None,
            pan: Vec2::new(80.0, 80.0),
            zoom: 1.0,
            dragged_port: None,
            chooser: None,
            count_edits: HashMap::new(),
            output_edits: HashMap::new(),
            next_spawn: 0,
            fit_requested: false,
        }
    }

    fn reload_data(&mut self) {
        let (data_root, result) = load_game_data();
        self.data_root = data_root;
        match result {
            Ok(data) => {
                self.load_message = format!(
                    "Loaded {} recipes and {} machines",
                    data.recipes.len(),
                    data.machines.len()
                );
                self.data = data;
            }
            Err(error) => self.load_message = error,
        }
    }

    fn add_recipe_at_default(&mut self, recipe_id: &str) {
        let col = self.next_spawn % 3;
        let row = self.next_spawn / 3;
        let position = Pos2::new(col as f32 * 360.0, row as f32 * 300.0);
        let id = self.plan.add_recipe(recipe_id, position, &self.data);
        self.selected_node = Some(id);
        self.next_spawn += 1;
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .exact_height(58.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("F")
                            .size(22.0)
                            .strong()
                            .color(Color32::BLACK)
                            .background_color(ORANGE),
                    );
                    ui.label(
                        RichText::new("FOUNDRY PLAN")
                            .size(18.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(format!(
                            "{} nodes  •  {} links",
                            self.plan.nodes.len(),
                            self.plan.edges.len()
                        ))
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Clear").clicked() {
                            self.plan = Plan::default();
                            self.selected_node = None;
                            self.count_edits.clear();
                            self.output_edits.clear();
                            self.next_spawn = 0;
                        }
                        let data_root_hover = self
                            .data_root
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| format!("Set {TEMPLATE_ROOT_ENV} and reload"));
                        if ui
                            .button("Reload data")
                            .on_hover_text(data_root_hover)
                            .clicked()
                        {
                            self.reload_data();
                        }
                        if ui.button("Fit plan").clicked() {
                            self.fit_requested = true;
                        }
                        ui.label(RichText::new("Drag ports to grow the plan").color(MUTED));
                    });
                });
            });
    }

    fn recipe_library(&mut self, ctx: &egui::Context) {
        let mut add = None;
        egui::SidePanel::left("recipes")
            .exact_width(SIDEBAR_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::same(14)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("RECIPE LIBRARY").strong().color(ORANGE));
                ui.add_space(7.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.recipe_search)
                        .hint_text("Search recipes or resources…")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(7.0);
                ui.label(RichText::new(&self.load_message).small().color(MUTED));
                ui.add_space(8.0);

                let needle = self.recipe_search.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for recipe in matching_recipes(&self.data, &needle) {
                            let machine_summary = recipe_machine_summary(recipe, &self.data);
                            let flow_summary = recipe_flow_summary(recipe, &self.data);
                            let response = egui::Frame::new()
                                .fill(CARD)
                                .corner_radius(6)
                                .inner_margin(Margin::same(10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.set_width(ui.available_width() - 30.0);
                                            ui.label(highlighted_text(
                                                &recipe.name,
                                                &needle,
                                                TextStyle::Body.resolve(ui.style()),
                                                TEXT,
                                            ));
                                            ui.label(
                                                RichText::new(machine_summary).small().color(MUTED),
                                            );
                                            ui.label(highlighted_text(
                                                &flow_summary,
                                                &needle,
                                                TextStyle::Small.resolve(ui.style()),
                                                CYAN,
                                            ));
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if ui
                                                    .small_button("＋")
                                                    .on_hover_text("Add to workspace")
                                                    .clicked()
                                                {
                                                    add = Some(recipe.id.clone());
                                                }
                                            },
                                        );
                                    });
                                })
                                .response
                                .interact(Sense::click());
                            if response.double_clicked() {
                                add = Some(recipe.id.clone());
                            }
                            ui.add_space(6.0);
                        }
                    });
            });
        if let Some(recipe_id) = add {
            self.add_recipe_at_default(&recipe_id);
        }
    }

    fn summary_panel(&mut self, ctx: &egui::Context, evaluation: &PlanEvaluation) {
        let totals = &evaluation.totals;
        egui::SidePanel::right("summary")
            .exact_width(SUMMARY_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::same(14)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("PLAN SUMMARY").strong().color(ORANGE));
                ui.add_space(10.0);
                egui::Frame::new()
                    .fill(CARD)
                    .corner_radius(6)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        let machines = if totals.has_values {
                            format_number(totals.machine_count)
                        } else {
                            "—".into()
                        };
                        ui.horizontal(|ui| {
                            let consumed = if totals.has_values {
                                format_power(totals.consumed_power_kw)
                            } else {
                                "—".into()
                            };
                            let generated = if totals.has_values {
                                format_power(totals.generated_power_kw)
                            } else {
                                "—".into()
                            };
                            metric(ui, "USED", &consumed, ORANGE);
                            ui.separator();
                            metric(ui, "MADE", &generated, GREEN);
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let net = if totals.has_values {
                                format_signed_power(totals.net_power_kw)
                            } else {
                                "—".into()
                            };
                            metric(ui, "NET", &net, CYAN);
                            ui.separator();
                            metric(ui, "MACHINES", &machines, CYAN);
                        });
                    });
                if !totals.has_values && !self.plan.nodes.is_empty() {
                    ui.label(
                        RichText::new("Enter a count or output rate to anchor the plan.")
                            .small()
                            .color(MUTED),
                    );
                } else if evaluation.unresolved_nodes > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} unresolved node{} excluded from totals.",
                            evaluation.unresolved_nodes,
                            if evaluation.unresolved_nodes == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ))
                        .small()
                        .color(MUTED),
                    );
                }
                ui.add_space(15.0);
                flow_section(
                    ui,
                    "EXTERNAL INPUTS",
                    &totals.inputs,
                    RED,
                    &self.data,
                    totals.has_values,
                    evaluation.unresolved_nodes > 0,
                );
                ui.add_space(14.0);
                flow_section(
                    ui,
                    "UNUSED OUTPUTS",
                    &totals.outputs,
                    GREEN,
                    &self.data,
                    totals.has_values,
                    evaluation.unresolved_nodes > 0,
                );

                ui.add_space(18.0);
                ui.separator();
                ui.add_space(12.0);
                ui.label(RichText::new("SELECTED MACHINE").strong().color(MUTED));
                ui.add_space(6.0);
                let Some(selected) = self.selected_node else {
                    ui.label(
                        RichText::new(
                            "Select a node to configure its machine, count, and clock speed.",
                        )
                        .color(MUTED),
                    );
                    return;
                };
                let Some(index) = self.plan.nodes.iter().position(|node| node.id == selected)
                else {
                    self.selected_node = None;
                    return;
                };
                let recipe_id = self.plan.nodes[index].recipe_id.clone();
                let Some(recipe) = self.data.recipe(&recipe_id) else {
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&recipe.name).strong().color(TEXT));
                    let pinned = self.plan.nodes[index].pinned_primary_rate.is_some();
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(if pinned { "PINNED" } else { "AUTO" })
                                .small()
                                .strong()
                                .color(if pinned { ORANGE } else { CYAN }),
                        );
                    });
                });
                let options: Vec<_> = self
                    .data
                    .machine_options(recipe)
                    .into_iter()
                    .map(|machine| (machine.id.clone(), machine.name.clone()))
                    .collect();
                if options.is_empty() {
                    ui.label(RichText::new("Manual / special crafting").color(MUTED));
                } else {
                    let current_name = self.plan.nodes[index]
                        .machine_id
                        .as_deref()
                        .and_then(|id| self.data.machine(id))
                        .map(|m| m.name.as_str())
                        .unwrap_or("Choose machine");
                    let mut selected_machine = self.plan.nodes[index].machine_id.clone();
                    egui::ComboBox::from_id_salt(("machine", selected))
                        .selected_text(current_name)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (machine_id, machine_name) in &options {
                                ui.selectable_value(
                                    &mut selected_machine,
                                    Some(machine_id.clone()),
                                    machine_name,
                                );
                            }
                        });
                    if selected_machine != self.plan.nodes[index].machine_id
                        && let Some(machine_id) = selected_machine
                    {
                        self.plan.set_machine(selected, machine_id, &self.data);
                    }
                }
                ui.add_space(6.0);
                let machine_kind = self.plan.nodes[index]
                    .machine_id
                    .as_deref()
                    .and_then(|id| self.data.machine(id))
                    .map(|machine| machine.kind.clone());
                match (&machine_kind, &mut self.plan.nodes[index].settings) {
                    (Some(MachineKind::Crafting), MachineSettings::Clock { percent }) => {
                        ui.horizontal(|ui| {
                            ui.label("Clock");
                            ui.add(
                                egui::DragValue::new(percent)
                                    .range(1.0..=250.0)
                                    .suffix("%")
                                    .speed(1.0),
                            );
                        });
                    }
                    (
                        Some(MachineKind::BlastFurnace(config)),
                        MachineSettings::BlastFurnace(settings),
                    ) => {
                        settings.towers =
                            settings.towers.clamp(config.min_towers, config.max_towers);
                        settings.temperature = settings
                            .temperature
                            .clamp(config.min_temperature, config.optimal_temperature);
                        ui.horizontal(|ui| {
                            ui.label("Towers");
                            ui.add(
                                egui::DragValue::new(&mut settings.towers)
                                    .range(config.min_towers..=config.max_towers)
                                    .speed(1.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Temperature");
                            ui.add(
                                egui::DragValue::new(&mut settings.temperature)
                                    .range(config.min_temperature..=config.optimal_temperature)
                                    .suffix("°C")
                                    .speed(5.0),
                            );
                        });
                    }
                    (
                        Some(MachineKind::ResourceConverter(config)),
                        MachineSettings::ResourceConverter { modules, adjacent },
                    ) => {
                        *modules = (*modules).clamp(config.min_modules, config.max_modules);
                        *adjacent = (*adjacent).min(config.max_adjacent);
                        if config.max_modules > 0 {
                            ui.horizontal(|ui| {
                                ui.label("Modules");
                                ui.add(
                                    egui::DragValue::new(modules)
                                        .range(config.min_modules..=config.max_modules)
                                        .speed(1.0),
                                );
                            });
                        }
                        if config.max_adjacent > 0 {
                            ui.horizontal(|ui| {
                                ui.label("Adjacent");
                                ui.add(
                                    egui::DragValue::new(adjacent)
                                        .range(0..=config.max_adjacent)
                                        .speed(1.0),
                                );
                            });
                        }
                    }
                    (
                        Some(MachineKind::EndlessMiner(config)),
                        MachineSettings::EndlessMiner { power_cores },
                    ) => {
                        *power_cores = (*power_cores).min(config.power_core_slots);
                        ui.horizontal(|ui| {
                            ui.label("Power cores");
                            ui.add(
                                egui::DragValue::new(power_cores)
                                    .range(0..=config.power_core_slots)
                                    .speed(1.0),
                            );
                        });
                    }
                    (
                        Some(MachineKind::Reactor),
                        MachineSettings::Reactor {
                            utilization_percent,
                        },
                    ) => {
                        *utilization_percent =
                            (*utilization_percent / 10.0).round().clamp(0.0, 10.0) * 10.0;
                        ui.horizontal(|ui| {
                            ui.label("Utilization");
                            ui.add(
                                egui::Slider::new(utilization_percent, 0.0..=100.0)
                                    .step_by(10.0)
                                    .suffix("%"),
                            );
                        });
                    }
                    (
                        Some(MachineKind::AssemblyLine(_)),
                        MachineSettings::AssemblyLine { painted },
                    ) => {
                        ui.checkbox(painted, "Painted finish");
                    }
                    _ => {}
                }
                let calculation = evaluation.node(selected).copied();
                let count_value = calculation.map(|value| value.machines);
                let output_value = calculation.map(|value| value.primary_rate);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Count");
                    let draft = self.count_edits.entry(selected).or_default();
                    if let Some(count) = optional_number_edit(
                        ui,
                        Id::new(("count_edit", selected)),
                        draft,
                        count_value,
                        78.0,
                    ) && let Some(rate) = primary_rate_for_machine_count(
                        &self.plan.nodes[index],
                        recipe,
                        count,
                        &self.data,
                    ) {
                        self.plan.nodes[index].pinned_primary_rate = Some(rate);
                    }
                    ui.label(
                        if matches!(machine_kind, Some(MachineKind::AssemblyLine(_))) {
                            "lines"
                        } else {
                            "machines"
                        },
                    );
                });
                let (anchor_side, anchor_slot) = recipe_rate_anchor(recipe);
                let primary = match anchor_side {
                    PortSide::Input => recipe.inputs.get(anchor_slot),
                    PortSide::Output => recipe.outputs.get(anchor_slot),
                };
                if let Some(primary) = primary {
                    let output_name = self.data.item_name(&primary.item);
                    ui.label(
                        RichText::new(format!(
                            "{} — {output_name}",
                            if anchor_side == PortSide::Input {
                                "Input"
                            } else {
                                "Output"
                            }
                        ))
                        .small()
                        .color(MUTED),
                    );
                    ui.horizontal(|ui| {
                        let draft = self.output_edits.entry(selected).or_default();
                        if let Some(rate) = optional_number_edit(
                            ui,
                            Id::new(("output_edit", selected)),
                            draft,
                            output_value,
                            96.0,
                        ) {
                            self.plan.nodes[index].pinned_primary_rate = Some(rate);
                        }
                        ui.label("/min");
                        if self.plan.nodes[index].pinned_primary_rate.is_some()
                            && ui.small_button("Unpin").clicked()
                        {
                            self.plan.nodes[index].pinned_primary_rate = None;
                            self.count_edits.remove(&selected);
                            self.output_edits.remove(&selected);
                        }
                    });
                }
                if let Some(machine) = self.plan.nodes[index]
                    .machine_id
                    .as_deref()
                    .and_then(|id| self.data.machine(id))
                {
                    if matches!(machine.kind, MachineKind::BlastFurnace(_)) {
                        let speed =
                            blast_furnace_operating_speed(&self.plan.nodes[index], &self.data)
                                .unwrap_or(0.0);
                        let hot_air =
                            blast_furnace_hot_air_rate(&self.plan.nodes[index], &self.data)
                                .unwrap_or(0.0);
                        ui.label(
                            RichText::new(format!(
                                "×{speed:.2} operating speed  •  {}/min hot air each",
                                format_rate(hot_air)
                            ))
                            .small()
                            .color(CYAN),
                        );
                    } else if let MachineSettings::Clock { percent } =
                        self.plan.nodes[index].settings
                    {
                        let factor = machine.speed * percent / 100.0;
                        ui.label(
                            RichText::new(format!(
                                "×{factor:.2} recipe speed  •  {} each",
                                format_power(machine.power_kw)
                            ))
                            .small()
                            .color(CYAN),
                        );
                    } else if let Some(required) = &machine.required_resource_node {
                        let name = self
                            .data
                            .resource_node_names
                            .get(required)
                            .cloned()
                            .unwrap_or_else(|| self.data.item_name(required));
                        ui.label(
                            RichText::new(format!("Requires {name}"))
                                .small()
                                .color(CYAN),
                        );
                    }
                }
            });
    }

    fn canvas(&mut self, ctx: &egui::Context, evaluation: &PlanEvaluation) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG))
            .show(ctx, |ui| {
                let canvas_rect = ui.max_rect();
                let canvas_response =
                    ui.interact(canvas_rect, Id::new("canvas"), Sense::click_and_drag());

                if canvas_response.dragged_by(PointerButton::Middle) {
                    self.pan += canvas_response.drag_delta();
                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                if canvas_response.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.01 {
                        let old_zoom = self.zoom;
                        self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.35, 1.8);
                        if let Some(mouse) = ctx.pointer_hover_pos() {
                            let world_at_mouse = (mouse - canvas_rect.min - self.pan) / old_zoom;
                            self.pan = mouse - canvas_rect.min - world_at_mouse * self.zoom;
                        }
                    }
                }
                if self.fit_requested {
                    self.fit_plan(canvas_rect);
                    self.fit_requested = false;
                }

                let painter = ui.painter_at(canvas_rect);
                draw_grid(&painter, canvas_rect, self.pan, self.zoom);

                for edge in &self.plan.edges {
                    let Some(from) = self.port_screen_position(&edge.from, canvas_rect) else {
                        continue;
                    };
                    let Some(to) = self.port_screen_position(&edge.to, canvas_rect) else {
                        continue;
                    };
                    let color = connection_color(evaluation.connection_state(&edge.from));
                    draw_connection(&painter, from, to, color, 2.5);
                }

                let pointer = ctx.pointer_hover_pos();
                let mut hovered_port: Option<PortRef> = None;
                let mut remove_node = None;
                let mut bring_to_front = None;
                let node_ids: Vec<_> = self.plan.nodes.iter().map(|node| node.id).collect();
                for node_id in node_ids {
                    let Some(index) = self.plan.nodes.iter().position(|node| node.id == node_id)
                    else {
                        continue;
                    };
                    let node = self.plan.nodes[index].clone();
                    let Some(recipe) = self.data.recipe(&node.recipe_id) else {
                        continue;
                    };
                    let size = node_size(recipe);
                    let min = world_to_screen(node.position, canvas_rect, self.pan, self.zoom);
                    let rect = Rect::from_min_size(min, size * self.zoom);
                    if !rect.intersects(canvas_rect.expand(50.0)) {
                        continue;
                    }

                    let selected = self.selected_node == Some(node.id);
                    let header = Rect::from_min_size(
                        rect.min,
                        Vec2::new(rect.width(), NODE_HEADER * self.zoom),
                    );
                    let drag = ui.interact(
                        header,
                        Id::new(("node_drag", node.id)),
                        Sense::click_and_drag(),
                    );
                    if drag.dragged() {
                        self.plan.nodes[index].position += drag.drag_delta() / self.zoom;
                        bring_to_front = Some(node.id);
                        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    if drag.clicked() {
                        self.selected_node = Some(node.id);
                        bring_to_front = Some(node.id);
                    }

                    draw_node(
                        &painter, rect, recipe, &node, &self.data, &self.plan, evaluation,
                        selected, self.zoom,
                    );

                    let close_rect = Rect::from_center_size(
                        Pos2::new(
                            rect.right() - 17.0 * self.zoom,
                            rect.top() + 18.0 * self.zoom,
                        ),
                        Vec2::splat(24.0 * self.zoom),
                    );
                    let close =
                        ui.interact(close_rect, Id::new(("remove", node.id)), Sense::click());
                    if close.clicked() {
                        remove_node = Some(node.id);
                    }
                    if close.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    for (side, ingredients) in [
                        (PortSide::Input, &recipe.inputs),
                        (PortSide::Output, &recipe.outputs),
                    ] {
                        for (slot, ingredient) in ingredients.iter().enumerate() {
                            let port = PortRef {
                                node: node.id,
                                side,
                                slot,
                                item: ingredient.item.clone(),
                            };
                            let center = self.port_screen_position(&port, canvas_rect).unwrap();
                            let hit = Rect::from_center_size(
                                center,
                                Vec2::splat(28.0 * self.zoom.max(0.7)),
                            );
                            let response = ui.interact(
                                hit,
                                Id::new(("port", node.id, side, slot)),
                                Sense::click_and_drag(),
                            );
                            if response.hovered() {
                                hovered_port = Some(port.clone());
                                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                            }
                            if response.drag_started() {
                                self.dragged_port = Some(port);
                                self.chooser = None;
                            }
                        }
                    }
                }

                if let Some(id) = bring_to_front
                    && let Some(index) = self.plan.nodes.iter().position(|node| node.id == id)
                {
                    let node = self.plan.nodes.remove(index);
                    self.plan.nodes.push(node);
                }
                if let Some(id) = remove_node {
                    self.plan.remove_node(id);
                    self.count_edits.remove(&id);
                    self.output_edits.remove(&id);
                    if self.selected_node == Some(id) {
                        self.selected_node = None;
                    }
                }

                if let (Some(origin), Some(cursor)) = (&self.dragged_port, pointer)
                    && let Some(start) = self.port_screen_position(origin, canvas_rect)
                {
                    let color = if hovered_port
                        .as_ref()
                        .is_some_and(|target| ports_compatible(origin, target))
                    {
                        GREEN
                    } else {
                        ORANGE
                    };
                    let (from, to) = if origin.side == PortSide::Output {
                        (start, cursor)
                    } else {
                        (cursor, start)
                    };
                    draw_connection(&painter, from, to, color, 3.0);
                }

                let primary_down = ctx.input(|i| i.pointer.primary_down());
                if self.dragged_port.is_some() && !primary_down {
                    let origin = self.dragged_port.take().unwrap();
                    if let Some(target) =
                        hovered_port.filter(|target| ports_compatible(&origin, target))
                    {
                        self.plan.connect(origin, target);
                    } else if let Some(screen_position) =
                        pointer.filter(|p| canvas_rect.contains(*p))
                    {
                        let has_choices = match origin.side {
                            PortSide::Input => {
                                !self.data.recipes_producing(&origin.item).is_empty()
                            }
                            PortSide::Output => {
                                !self.data.recipes_consuming(&origin.item).is_empty()
                            }
                        };
                        if has_choices {
                            self.chooser = Some(RecipeChooser {
                                origin,
                                screen_position,
                                world_position: screen_to_world(
                                    screen_position,
                                    canvas_rect,
                                    self.pan,
                                    self.zoom,
                                ),
                                search: String::new(),
                            });
                        }
                    }
                }

                if self.plan.nodes.is_empty() {
                    painter.text(
                        canvas_rect.center() - Vec2::new(0.0, 18.0),
                        Align2::CENTER_CENTER,
                        "Build your factory",
                        FontId::proportional(26.0),
                        TEXT,
                    );
                    painter.text(
                        canvas_rect.center() + Vec2::new(0.0, 18.0),
                        Align2::CENTER_CENTER,
                        "Add a recipe from the library, then drag its colored ports.",
                        FontId::proportional(15.0),
                        MUTED,
                    );
                }
                painter.text(
                    canvas_rect.left_bottom() + Vec2::new(14.0, -12.0),
                    Align2::LEFT_BOTTOM,
                    format!(
                        "{}%  •  Middle-drag to pan  •  Scroll to zoom",
                        (self.zoom * 100.0).round()
                    ),
                    FontId::proportional(12.0),
                    MUTED,
                );
            });
    }

    fn recipe_chooser(&mut self, ctx: &egui::Context) {
        let Some(snapshot) = self.chooser.clone() else {
            return;
        };
        let title = match snapshot.origin.side {
            PortSide::Input => format!("Produce {}", self.data.item_name(&snapshot.origin.item)),
            PortSide::Output => format!("Use {}", self.data.item_name(&snapshot.origin.item)),
        };
        let mut chosen: Option<String> = None;
        let mut close = false;
        egui::Window::new(title)
            .id(Id::new("recipe_chooser"))
            .fixed_pos(snapshot.screen_position)
            .default_width(330.0)
            .collapsible(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.style()).fill(PANEL))
            .show(ctx, |ui| {
                let chooser = self.chooser.as_mut().unwrap();
                ui.add(
                    egui::TextEdit::singleline(&mut chooser.search)
                        .hint_text("Filter compatible recipes…")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(6.0);
                let needle = chooser.search.to_lowercase();
                let candidates = match snapshot.origin.side {
                    PortSide::Input => self.data.recipes_producing(&snapshot.origin.item),
                    PortSide::Output => self.data.recipes_consuming(&snapshot.origin.item),
                };
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for recipe in candidates {
                            if !recipe_matches(recipe, &needle, &self.data) {
                                continue;
                            }
                            let machine_summary = recipe_machine_summary(recipe, &self.data);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!(
                                            "{}\n{}  •  {}",
                                            recipe.name,
                                            machine_summary,
                                            recipe_flow_summary(recipe, &self.data)
                                        ))
                                        .color(TEXT),
                                    )
                                    .fill(CARD)
                                    .min_size(Vec2::new(ui.available_width(), 48.0)),
                                )
                                .clicked()
                            {
                                chosen = Some(recipe.id.clone());
                            }
                        }
                    });
                ui.add_space(5.0);
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if let Some(recipe_id) = chosen {
            self.add_connected_recipe(&recipe_id, snapshot);
            close = true;
        }
        if close {
            self.chooser = None;
        }
    }

    fn add_connected_recipe(&mut self, recipe_id: &str, chooser: RecipeChooser) {
        let Some(recipe) = self.data.recipe(recipe_id) else {
            return;
        };
        let position = match chooser.origin.side {
            PortSide::Input => chooser.world_position - Vec2::new(NODE_WIDTH, NODE_HEADER),
            PortSide::Output => chooser.world_position - Vec2::new(0.0, NODE_HEADER),
        };
        let matching_slot = match chooser.origin.side {
            PortSide::Input => recipe
                .outputs
                .iter()
                .position(|i| i.item == chooser.origin.item)
                .map(|slot| (PortSide::Output, slot)),
            PortSide::Output => recipe
                .inputs
                .iter()
                .position(|i| i.item == chooser.origin.item)
                .map(|slot| (PortSide::Input, slot)),
        };
        let Some((side, slot)) = matching_slot else {
            return;
        };
        let item = chooser.origin.item.clone();
        let new_id = self.plan.add_recipe(recipe_id, position, &self.data);
        let new_port = PortRef {
            node: new_id,
            side,
            slot,
            item,
        };
        self.plan.connect(chooser.origin, new_port);
        self.selected_node = Some(new_id);
    }

    fn port_screen_position(&self, port: &PortRef, canvas: Rect) -> Option<Pos2> {
        let node = self.plan.nodes.iter().find(|node| node.id == port.node)?;
        let recipe = self.data.recipe(&node.recipe_id)?;
        let y = node.position.y + NODE_HEADER + port.slot as f32 * PORT_ROW + PORT_ROW * 0.5;
        let x = node.position.x
            + if port.side == PortSide::Output {
                NODE_WIDTH
            } else {
                0.0
            };
        if (port.side == PortSide::Input && port.slot >= recipe.inputs.len())
            || (port.side == PortSide::Output && port.slot >= recipe.outputs.len())
        {
            return None;
        }
        Some(world_to_screen(
            Pos2::new(x, y),
            canvas,
            self.pan,
            self.zoom,
        ))
    }

    fn fit_plan(&mut self, canvas: Rect) {
        if self.plan.nodes.is_empty() {
            self.zoom = 1.0;
            self.pan = Vec2::new(80.0, 80.0);
            return;
        }
        let mut bounds = Rect::NOTHING;
        for node in &self.plan.nodes {
            if let Some(recipe) = self.data.recipe(&node.recipe_id) {
                bounds = bounds.union(Rect::from_min_size(node.position, node_size(recipe)));
            }
        }
        let padding = 70.0;
        let available = canvas.size() - Vec2::splat(padding * 2.0);
        self.zoom = (available.x / bounds.width().max(1.0))
            .min(available.y / bounds.height().max(1.0))
            .clamp(0.35, 1.4);
        self.pan = canvas.center().to_vec2()
            - canvas.min.to_vec2()
            - bounds.center().to_vec2() * self.zoom;
    }
}

impl eframe::App for PlannerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_bar(ctx);
        self.recipe_library(ctx);
        let evaluation = self.plan.evaluate(&self.data);
        self.summary_panel(ctx, &evaluation);
        let evaluation = self.plan.evaluate(&self.data);
        self.canvas(ctx, &evaluation);
        self.recipe_chooser(ctx);

        if node_delete_requested(ctx)
            && let Some(id) = self.selected_node.take()
        {
            self.plan.remove_node(id);
            self.count_edits.remove(&id);
            self.output_edits.remove(&id);
        }
        ctx.request_repaint();
    }
}

fn node_delete_requested(ctx: &egui::Context) -> bool {
    !ctx.wants_keyboard_input()
        && ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = Color32::from_rgb(10, 17, 22);
    style.visuals.widgets.inactive.bg_fill = CARD_ALT;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(46, 62, 72);
    style.visuals.widgets.active.bg_fill = ORANGE;
    style.visuals.selection.bg_fill = ORANGE;
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    ctx.set_style(style);
}

fn node_size(recipe: &Recipe) -> Vec2 {
    let rows = recipe.inputs.len().max(recipe.outputs.len()).max(1) as f32;
    Vec2::new(NODE_WIDTH, NODE_HEADER + rows * PORT_ROW + NODE_FOOTER)
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    painter: &egui::Painter,
    rect: Rect,
    recipe: &Recipe,
    node: &crate::model::PlanNode,
    data: &GameData,
    plan: &Plan,
    evaluation: &PlanEvaluation,
    selected: bool,
    zoom: f32,
) {
    let rounding = CornerRadius::same((8.0 * zoom).clamp(2.0, 12.0) as u8);
    painter.rect_filled(rect, rounding, CARD);
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(
            if selected { 2.5_f32 } else { 1.0_f32 },
            if selected {
                ORANGE
            } else {
                Color32::from_rgb(57, 76, 87)
            },
        ),
        StrokeKind::Inside,
    );
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), NODE_HEADER * zoom));
    painter.rect_filled(header, rounding, CARD_ALT);
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(header.left(), header.bottom() - 5.0 * zoom),
            header.right_bottom(),
        ),
        CornerRadius::ZERO,
        ORANGE,
    );
    let operating_summary = match &node.settings {
        MachineSettings::Clock { percent } => format!("{percent:.0}%"),
        MachineSettings::BlastFurnace(settings) => {
            format!("{} towers • {:.0}°C", settings.towers, settings.temperature)
        }
        MachineSettings::ResourceConverter { modules, adjacent } => {
            format!("{modules} modules • {adjacent} adjacent")
        }
        MachineSettings::EndlessMiner { power_cores } => format!("{power_cores} cores"),
        MachineSettings::Reactor {
            utilization_percent,
        } => format!("{utilization_percent:.0}%"),
        MachineSettings::AssemblyLine { painted } => {
            if *painted { "painted" } else { "metal" }.to_owned()
        }
        MachineSettings::Fixed => "fixed".to_owned(),
    };
    painter.text(
        header.left_top() + Vec2::new(14.0, 12.0) * zoom,
        Align2::LEFT_TOP,
        &recipe.name,
        FontId::proportional(16.0 * zoom),
        TEXT,
    );
    let machine = node.machine_id.as_deref().and_then(|id| data.machine(id));
    let machine_name = machine
        .map(|m| m.name.as_str())
        .unwrap_or("Manual / special crafting");
    painter.text(
        header.left_top() + Vec2::new(14.0, 35.0) * zoom,
        Align2::LEFT_TOP,
        machine_name,
        FontId::proportional(12.0 * zoom),
        MUTED,
    );
    painter.text(
        header.right_top() + Vec2::new(-17.0, 17.0) * zoom,
        Align2::CENTER_CENTER,
        "×",
        FontId::proportional(17.0 * zoom),
        MUTED,
    );

    for (side, ingredients) in [
        (PortSide::Input, &recipe.inputs),
        (PortSide::Output, &recipe.outputs),
    ] {
        for (slot, ingredient) in ingredients.iter().enumerate() {
            let y = rect.top() + (NODE_HEADER + slot as f32 * PORT_ROW + PORT_ROW * 0.5) * zoom;
            let x = if side == PortSide::Input {
                rect.left()
            } else {
                rect.right()
            };
            let port = PortRef {
                node: node.id,
                side,
                slot,
                item: ingredient.item.clone(),
            };
            let connected = plan.is_connected(&port);
            let color = if connected {
                connection_color(evaluation.port_connection_state(&port, plan))
            } else if side == PortSide::Input {
                RED
            } else {
                GREEN
            };
            painter.circle_filled(Pos2::new(x, y), PORT_RADIUS * zoom.max(0.72), color);
            painter.circle_stroke(
                Pos2::new(x, y),
                (PORT_RADIUS + 2.0) * zoom.max(0.72),
                Stroke::new(1.0_f32, color.gamma_multiply(0.45)),
            );
            let item_name = data.item_name(&ingredient.item);
            let (pos, align) = if side == PortSide::Input {
                (Pos2::new(x + 15.0 * zoom, y), Align2::LEFT_CENTER)
            } else {
                (Pos2::new(x - 15.0 * zoom, y), Align2::RIGHT_CENTER)
            };
            painter.text(
                pos,
                align,
                item_name,
                FontId::proportional(12.5 * zoom),
                TEXT,
            );
            if let Some(rate) = evaluation.port_rate(&port, plan, data) {
                let rate_pos = pos + Vec2::new(0.0, 15.0 * zoom);
                painter.text(
                    rate_pos,
                    align,
                    format!("{}/min", format_rate(rate)),
                    FontId::proportional(10.5 * zoom),
                    color,
                );
            }
        }
    }
    let footer_y = rect.bottom() - 24.0 * zoom;
    let calculation = evaluation.node(node.id);
    let status = if node.pinned_primary_rate.is_some() {
        "PINNED"
    } else {
        "AUTO"
    };
    let count = calculation
        .map(|value| format_number(value.machines))
        .unwrap_or_else(|| "—".into());
    painter.text(
        Pos2::new(rect.left() + 14.0 * zoom, footer_y),
        Align2::LEFT_CENTER,
        format!("{status}  •  {count} ×  {operating_summary}"),
        FontId::proportional(11.5 * zoom),
        MUTED,
    );
    painter.text(
        Pos2::new(rect.right() - 14.0 * zoom, footer_y),
        Align2::RIGHT_CENTER,
        calculation
            .map(format_node_power)
            .unwrap_or_else(|| "—".into()),
        FontId::proportional(11.5 * zoom),
        if calculation.is_some_and(|value| value.generated_power_kw > 0.0) {
            GREEN
        } else {
            ORANGE
        },
    );
}

fn draw_grid(painter: &egui::Painter, rect: Rect, pan: Vec2, zoom: f32) {
    let spacing = 32.0 * zoom;
    if spacing < 8.0 {
        return;
    }
    let offset_x = pan.x.rem_euclid(spacing);
    let offset_y = pan.y.rem_euclid(spacing);
    let color = Color32::from_rgba_unmultiplied(72, 94, 105, 35);
    let mut x = rect.left() + offset_x;
    while x < rect.right() {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0_f32, color),
        );
        x += spacing;
    }
    let mut y = rect.top() + offset_y;
    while y < rect.bottom() {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0_f32, color),
        );
        y += spacing;
    }
}

fn draw_connection(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32, width: f32) {
    let control = ((to.x - from.x).abs() * 0.5).max(65.0);
    let points = [
        from,
        from + Vec2::new(control, 0.0),
        to - Vec2::new(control, 0.0),
        to,
    ];
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        points,
        false,
        Color32::TRANSPARENT,
        Stroke::new(width, color),
    ));
}

fn world_to_screen(world: Pos2, canvas: Rect, pan: Vec2, zoom: f32) -> Pos2 {
    canvas.min + pan + world.to_vec2() * zoom
}

fn screen_to_world(screen: Pos2, canvas: Rect, pan: Vec2, zoom: f32) -> Pos2 {
    Pos2::new(
        (screen.x - canvas.min.x - pan.x) / zoom,
        (screen.y - canvas.min.y - pan.y) / zoom,
    )
}

fn ports_compatible(a: &PortRef, b: &PortRef) -> bool {
    a.node != b.node && a.side != b.side && a.item == b.item
}

fn connection_color(state: ConnectionState) -> Color32 {
    match state {
        ConnectionState::Balanced => CYAN,
        ConnectionState::Partial => ORANGE,
        ConnectionState::Unresolved => MUTED,
    }
}

fn optional_number_edit(
    ui: &mut egui::Ui,
    id: Id,
    draft: &mut String,
    current: Option<f32>,
    width: f32,
) -> Option<f32> {
    let focused = ui.memory(|memory| memory.has_focus(id));
    if !focused {
        *draft = current.map(format_edit_number).unwrap_or_default();
    }
    let response = ui.add(
        egui::TextEdit::singleline(draft)
            .id(id)
            .hint_text("—")
            .desired_width(width),
    );
    if !response.changed() {
        return None;
    }
    draft
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn format_edit_number(value: f32) -> String {
    let formatted = format!("{value:.3}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn recipe_matches(recipe: &Recipe, needle: &str, data: &GameData) -> bool {
    recipe_match_rank(recipe, needle, data).is_some()
}

fn matching_recipes<'a>(data: &'a GameData, needle: &str) -> Vec<&'a Recipe> {
    let mut matches = data
        .recipes
        .iter()
        .filter_map(|recipe| recipe_match_rank(recipe, needle, data).map(|rank| (rank, recipe)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|(rank, _)| *rank);
    matches.into_iter().map(|(_, recipe)| recipe).collect()
}

fn recipe_match_rank(recipe: &Recipe, needle: &str, data: &GameData) -> Option<u8> {
    if needle.is_empty() {
        return Some(0);
    }

    match_quality(&recipe.name, needle)
        .or_else(|| {
            recipe
                .outputs
                .iter()
                .filter_map(|ingredient| match_quality(&data.item_name(&ingredient.item), needle))
                .min()
                .map(|quality| 3 + quality)
        })
        .or_else(|| {
            recipe
                .inputs
                .iter()
                .filter_map(|ingredient| match_quality(&data.item_name(&ingredient.item), needle))
                .min()
                .map(|quality| 6 + quality)
        })
        .or_else(|| match_quality(&recipe.category, needle).map(|quality| 9 + quality))
        .or_else(|| match_quality(&recipe.id, needle).map(|quality| 12 + quality))
}

fn match_quality(text: &str, needle: &str) -> Option<u8> {
    let text = text.to_lowercase();
    if text == needle {
        Some(0)
    } else if text.starts_with(needle) {
        Some(1)
    } else if text.contains(needle) {
        Some(2)
    } else {
        None
    }
}

fn highlighted_text(text: &str, needle: &str, font_id: FontId, color: Color32) -> LayoutJob {
    let normal = TextFormat {
        font_id,
        color,
        ..Default::default()
    };
    let highlighted = TextFormat {
        color: Color32::BLACK,
        background: ORANGE,
        ..normal.clone()
    };
    let mut job = LayoutJob::default();
    let mut cursor = 0;

    for range in case_insensitive_match_ranges(text, needle) {
        job.append(&text[cursor..range.start], 0.0, normal.clone());
        job.append(&text[range.clone()], 0.0, highlighted.clone());
        cursor = range.end;
    }
    job.append(&text[cursor..], 0.0, normal);
    job
}

fn case_insensitive_match_ranges(text: &str, needle: &str) -> Vec<std::ops::Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }

    let mut folded = String::new();
    let mut character_ranges = Vec::new();
    for (start, character) in text.char_indices() {
        let folded_start = folded.len();
        folded.extend(character.to_lowercase());
        character_ranges.push((
            folded_start..folded.len(),
            start..start + character.len_utf8(),
        ));
    }

    folded
        .match_indices(needle)
        .filter_map(|(start, matched)| {
            let end = start + matched.len();
            let original_start = character_ranges
                .iter()
                .find(|(folded_range, _)| folded_range.end > start)?
                .1
                .start;
            let original_end = character_ranges
                .iter()
                .rev()
                .find(|(folded_range, _)| folded_range.start < end)?
                .1
                .end;
            Some(original_start..original_end)
        })
        .collect()
}

fn recipe_flow_summary(recipe: &Recipe, data: &GameData) -> String {
    let inputs = recipe
        .inputs
        .iter()
        .map(|i| data.item_name(&i.item))
        .collect::<Vec<_>>()
        .join(" + ");
    let outputs = recipe
        .outputs
        .iter()
        .map(|i| data.item_name(&i.item))
        .collect::<Vec<_>>()
        .join(" + ");
    format!(
        "{} → {}",
        if inputs.is_empty() { "—" } else { &inputs },
        if outputs.is_empty() { "—" } else { &outputs }
    )
}

fn recipe_machine_summary(recipe: &Recipe, data: &GameData) -> String {
    let machine = data
        .machine_options(recipe)
        .first()
        .map(|machine| machine.name.as_str())
        .unwrap_or("Manual / special");
    match recipe.kind {
        RecipeKind::BlastFurnace { .. } => format!("{machine}  •  configurable"),
        RecipeKind::Crafting => format!("{machine}  •  {:.1}s", recipe.time_seconds),
        RecipeKind::Direct { .. } => format!("{machine}  •  direct"),
    }
}

fn metric(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).small().color(MUTED));
        ui.label(RichText::new(value).size(20.0).strong().color(color));
    });
}

fn flow_section(
    ui: &mut egui::Ui,
    title: &str,
    values: &HashMap<String, f32>,
    color: Color32,
    data: &GameData,
    has_values: bool,
    has_unresolved: bool,
) {
    ui.label(RichText::new(title).strong().color(color));
    ui.add_space(5.0);
    if values.is_empty() {
        ui.label(
            RichText::new(if !has_values {
                "—"
            } else if has_unresolved {
                "None among resolved nodes"
            } else {
                "None — all accounted for"
            })
            .small()
            .color(MUTED),
        );
        return;
    }
    let mut rows: Vec<_> = values.iter().collect();
    rows.sort_by_key(|(id, _)| data.item_name(id).to_lowercase());
    for (item, rate) in rows {
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(color));
            ui.label(RichText::new(data.item_name(item)).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{}/m", format_rate(*rate)))
                        .strong()
                        .color(color),
                );
            });
        });
    }
}

fn format_rate(value: f32) -> String {
    if (value - value.round()).abs() < 0.02 {
        format!("{:.0}", value)
    } else {
        format!("{:.1}", value)
    }
}

fn format_number(value: f32) -> String {
    if (value - value.round()).abs() < 0.02 {
        format!("{:.0}", value)
    } else {
        format!("{:.1}", value)
    }
}

fn format_power(kw: f32) -> String {
    if kw >= 1000.0 {
        format!("{:.2} MW", kw / 1000.0)
    } else {
        format!("{:.0} kW", kw)
    }
}

fn format_signed_power(kw: f32) -> String {
    let sign = if kw > 0.01 {
        "+"
    } else if kw < -0.01 {
        "−"
    } else {
        ""
    };
    format!("{sign}{}", format_power(kw.abs()))
}

fn format_node_power(value: &crate::model::NodeCalculation) -> String {
    if value.generated_power_kw > 0.0 {
        format!("+{}", format_power(value.generated_power_kw))
    } else {
        format_power(value.consumed_power_kw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_recipe(id: &str, name: &str, input: Option<&str>, output: &str) -> Recipe {
        Recipe {
            id: id.to_owned(),
            name: name.to_owned(),
            inputs: input
                .map(|item| {
                    vec![crate::data::Ingredient {
                        item: item.to_owned(),
                        amount: 1.0,
                    }]
                })
                .unwrap_or_default(),
            outputs: vec![crate::data::Ingredient {
                item: output.to_owned(),
                amount: 1.0,
            }],
            time_seconds: 1.0,
            tags: Vec::new(),
            category: String::new(),
            kind: RecipeKind::Crafting,
        }
    }

    fn key_input(key: Key) -> egui::RawInput {
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        input
    }

    fn delete_requested_for(key: Key, keyboard_focused: bool) -> bool {
        let ctx = egui::Context::default();
        let mut requested = false;
        let _ = ctx.run(key_input(key), |ctx| {
            if keyboard_focused {
                ctx.memory_mut(|memory| memory.request_focus(Id::new("focused_editor")));
            }
            requested = node_delete_requested(ctx);
        });
        requested
    }

    #[test]
    fn keyboard_focus_suppresses_node_deletion_shortcuts() {
        assert!(!delete_requested_for(Key::Backspace, true));
        assert!(!delete_requested_for(Key::Delete, true));
    }

    #[test]
    fn node_deletion_shortcuts_work_without_keyboard_focus() {
        assert!(delete_requested_for(Key::Backspace, false));
        assert!(delete_requested_for(Key::Delete, false));
    }

    #[test]
    fn unrelated_keys_do_not_request_node_deletion() {
        assert!(!delete_requested_for(Key::Enter, false));
    }

    #[test]
    fn recipe_name_matches_rank_ahead_of_input_only_matches() {
        let mut data = GameData::from_test_parts(
            vec![
                test_recipe(
                    "assembly_line_rail",
                    "Assembly Line Rail",
                    Some("xenoferrite_plates"),
                    "rail",
                ),
                test_recipe(
                    "plate_recipe",
                    "Xenoferrite Plates",
                    Some("molten_xenoferrite"),
                    "xenoferrite_plates",
                ),
            ],
            Vec::new(),
        );
        data.item_names.insert(
            "xenoferrite_plates".to_owned(),
            "Xenoferrite Plates".to_owned(),
        );
        data.item_names.insert(
            "molten_xenoferrite".to_owned(),
            "Molten Xenoferrite".to_owned(),
        );

        let matches = matching_recipes(&data, "xenoferrite");

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].id, "plate_recipe");
        assert_eq!(matches[1].id, "assembly_line_rail");
    }

    #[test]
    fn highlight_ranges_are_case_insensitive_and_utf8_safe() {
        assert_eq!(
            case_insensitive_match_ranges("Xenoferrite Plates", "ferr"),
            vec![4..8]
        );
        assert_eq!(case_insensitive_match_ranges("İnput", "i"), vec![0..2]);
    }
}

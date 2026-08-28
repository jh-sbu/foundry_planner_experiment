use std::{
    collections::HashSet,
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
};

use eframe::egui::{Pos2, Vec2};
use serde::{Deserialize, Serialize};

use crate::{
    data::{GameData, Machine, MachineKind},
    model::{
        BlastFurnaceSettings, Edge, MachineSettings, NodeId, Plan, PlanNode, PortRef, PortSide,
    },
};

pub const PLAN_FILE_EXTENSION: &str = "foundry-plan";
const PLAN_FILE_VERSION: u32 = 1;
const MIN_ZOOM: f32 = 0.35;
const MAX_ZOOM: f32 = 1.8;

pub struct LoadedPlan {
    pub plan: Plan,
    pub pan: Vec2,
    pub zoom: f32,
}

#[derive(Debug)]
pub enum PlanFileError {
    Read { path: PathBuf, error: String },
    Write { path: PathBuf, error: String },
    InvalidJson(String),
    UnsupportedVersion(u32),
    InvalidPlan(String),
}

impl fmt::Display for PlanFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => {
                write!(formatter, "Could not read {}: {error}", path.display())
            }
            Self::Write { path, error } => {
                write!(formatter, "Could not save {}: {error}", path.display())
            }
            Self::InvalidJson(error) => write!(formatter, "Invalid plan file: {error}"),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "This plan uses unsupported format version {version} (expected {PLAN_FILE_VERSION})"
            ),
            Self::InvalidPlan(error) => write!(formatter, "Invalid plan: {error}"),
        }
    }
}

impl std::error::Error for PlanFileError {}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanFileV1 {
    version: u32,
    view: SavedView,
    nodes: Vec<SavedNode>,
    edges: Vec<SavedEdge>,
}

#[derive(Deserialize)]
struct VersionHeader {
    version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedView {
    pan: [f32; 2],
    zoom: f32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedNode {
    id: NodeId,
    recipe_id: String,
    machine_id: Option<String>,
    position: [f32; 2],
    pinned_primary_rate: Option<f32>,
    settings: SavedMachineSettings,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum SavedMachineSettings {
    Clock { percent: f32 },
    BlastFurnace { towers: u32, temperature: f32 },
    ResourceConverter { modules: u32, adjacent: u32 },
    EndlessMiner { power_cores: u32 },
    Reactor { utilization_percent: f32 },
    AssemblyLine { painted: bool },
    Fixed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedEdge {
    from: SavedPort,
    to: SavedPort,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPort {
    node: NodeId,
    side: SavedPortSide,
    slot: usize,
    item: String,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedPortSide {
    Input,
    Output,
}

pub fn save_plan(path: &Path, plan: &Plan, pan: Vec2, zoom: f32) -> Result<(), PlanFileError> {
    let json = encode_plan(plan, pan, zoom)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| PlanFileError::Write {
            path: path.to_path_buf(),
            error: error.to_string(),
        })?;
    temporary
        .write_all(json.as_bytes())
        .and_then(|()| temporary.flush())
        .map_err(|error| PlanFileError::Write {
            path: path.to_path_buf(),
            error: error.to_string(),
        })?;
    temporary
        .persist(path)
        .map_err(|error| PlanFileError::Write {
            path: path.to_path_buf(),
            error: error.error.to_string(),
        })?;
    Ok(())
}

pub fn load_plan(path: &Path, data: &GameData) -> Result<LoadedPlan, PlanFileError> {
    let json = fs::read_to_string(path).map_err(|error| PlanFileError::Read {
        path: path.to_path_buf(),
        error: error.to_string(),
    })?;
    decode_plan(&json, data)
}

fn encode_plan(plan: &Plan, pan: Vec2, zoom: f32) -> Result<String, PlanFileError> {
    validate_view([pan.x, pan.y], zoom)?;
    let file = PlanFileV1 {
        version: PLAN_FILE_VERSION,
        view: SavedView {
            pan: [pan.x, pan.y],
            zoom,
        },
        nodes: plan.nodes.iter().map(SavedNode::from).collect(),
        edges: plan.edges.iter().map(SavedEdge::from).collect(),
    };
    serde_json::to_string_pretty(&file)
        .map_err(|error| PlanFileError::InvalidJson(error.to_string()))
}

fn decode_plan(json: &str, data: &GameData) -> Result<LoadedPlan, PlanFileError> {
    let header: VersionHeader = serde_json::from_str(json)
        .map_err(|error| PlanFileError::InvalidJson(error.to_string()))?;
    if header.version != PLAN_FILE_VERSION {
        return Err(PlanFileError::UnsupportedVersion(header.version));
    }
    let file: PlanFileV1 = serde_json::from_str(json)
        .map_err(|error| PlanFileError::InvalidJson(error.to_string()))?;
    validate_view(file.view.pan, file.view.zoom)?;

    let mut ids = HashSet::new();
    let mut nodes = Vec::with_capacity(file.nodes.len());
    for saved in file.nodes {
        if saved.id == 0 {
            return invalid("node IDs must be greater than zero");
        }
        if saved.id == NodeId::MAX {
            return invalid("the maximum node ID cannot be restored safely");
        }
        if !ids.insert(saved.id) {
            return invalid(format!("duplicate node ID {}", saved.id));
        }
        if !saved.position.iter().all(|value| value.is_finite()) {
            return invalid(format!("node {} has a non-finite position", saved.id));
        }
        if saved
            .pinned_primary_rate
            .is_some_and(|rate| !rate.is_finite() || rate < 0.0)
        {
            return invalid(format!("node {} has an invalid pinned rate", saved.id));
        }
        let recipe = data.recipe(&saved.recipe_id).ok_or_else(|| {
            PlanFileError::InvalidPlan(format!(
                "node {} references missing recipe '{}'",
                saved.id, saved.recipe_id
            ))
        })?;
        let machine = match &saved.machine_id {
            Some(machine_id) => {
                let machine = data.machine(machine_id).ok_or_else(|| {
                    PlanFileError::InvalidPlan(format!(
                        "node {} references missing machine '{}'",
                        saved.id, machine_id
                    ))
                })?;
                if !data
                    .machine_options(recipe)
                    .iter()
                    .any(|option| option.id == machine.id)
                {
                    return invalid(format!(
                        "machine '{}' cannot run recipe '{}' on node {}",
                        machine.id, recipe.id, saved.id
                    ));
                }
                Some(machine)
            }
            None => {
                if !data.machine_options(recipe).is_empty() {
                    return invalid(format!(
                        "node {} is missing a machine for recipe '{}'",
                        saved.id, recipe.id
                    ));
                }
                None
            }
        };
        let settings = MachineSettings::from(saved.settings);
        validate_settings(saved.id, machine, &settings)?;
        nodes.push(PlanNode {
            id: saved.id,
            recipe_id: saved.recipe_id,
            machine_id: saved.machine_id,
            position: Pos2::new(saved.position[0], saved.position[1]),
            pinned_primary_rate: saved.pinned_primary_rate,
            settings,
        });
    }

    let mut edges = Vec::with_capacity(file.edges.len());
    let mut unique_edges = HashSet::new();
    let mut connected_inputs = HashSet::new();
    for saved in file.edges {
        let from = PortRef::from(saved.from);
        let to = PortRef::from(saved.to);
        validate_port(&from, &nodes, data)?;
        validate_port(&to, &nodes, data)?;
        if from.side != PortSide::Output || to.side != PortSide::Input {
            return invalid("every edge must run from an output port to an input port");
        }
        if from.node == to.node {
            return invalid(format!("node {} contains a self-connection", from.node));
        }
        if from.item != to.item {
            return invalid(format!(
                "edge from node {} to node {} connects different items",
                from.node, to.node
            ));
        }
        if !unique_edges.insert((from.clone(), to.clone())) {
            return invalid(format!(
                "duplicate edge from node {} to node {}",
                from.node, to.node
            ));
        }
        if !connected_inputs.insert(to.clone()) {
            return invalid(format!(
                "input {} on node {} has more than one connection",
                to.slot, to.node
            ));
        }
        edges.push(Edge { from, to });
    }

    Ok(LoadedPlan {
        plan: Plan::from_parts(nodes, edges),
        pan: Vec2::new(file.view.pan[0], file.view.pan[1]),
        zoom: file.view.zoom,
    })
}

fn validate_view(pan: [f32; 2], zoom: f32) -> Result<(), PlanFileError> {
    if !pan.iter().all(|value| value.is_finite()) {
        return invalid("the saved camera position is not finite");
    }
    if !zoom.is_finite() || !(MIN_ZOOM..=MAX_ZOOM).contains(&zoom) {
        return invalid(format!(
            "the saved zoom must be between {MIN_ZOOM} and {MAX_ZOOM}"
        ));
    }
    Ok(())
}

fn validate_port(port: &PortRef, nodes: &[PlanNode], data: &GameData) -> Result<(), PlanFileError> {
    let node = nodes
        .iter()
        .find(|node| node.id == port.node)
        .ok_or_else(|| {
            PlanFileError::InvalidPlan(format!("edge references missing node {}", port.node))
        })?;
    let recipe = data.recipe(&node.recipe_id).expect("nodes were validated");
    let ingredient = match port.side {
        PortSide::Input => recipe.inputs.get(port.slot),
        PortSide::Output => recipe.outputs.get(port.slot),
    }
    .ok_or_else(|| {
        PlanFileError::InvalidPlan(format!(
            "edge references missing {:?} slot {} on node {}",
            port.side, port.slot, port.node
        ))
    })?;
    if ingredient.item != port.item {
        return invalid(format!(
            "edge item '{}' does not match {:?} slot {} on node {}",
            port.item, port.side, port.slot, port.node
        ));
    }
    Ok(())
}

fn validate_settings(
    node_id: NodeId,
    machine: Option<&Machine>,
    settings: &MachineSettings,
) -> Result<(), PlanFileError> {
    let valid = match (machine.map(|machine| &machine.kind), settings) {
        (None, MachineSettings::Clock { percent }) => {
            percent.is_finite() && (1.0..=250.0).contains(percent)
        }
        (Some(MachineKind::Crafting), MachineSettings::Clock { percent }) => {
            percent.is_finite() && (1.0..=250.0).contains(percent)
        }
        (Some(MachineKind::BlastFurnace(config)), MachineSettings::BlastFurnace(settings)) => {
            (config.min_towers..=config.max_towers).contains(&settings.towers)
                && settings.temperature.is_finite()
                && (config.min_temperature..=config.optimal_temperature)
                    .contains(&settings.temperature)
        }
        (
            Some(MachineKind::ResourceConverter(config)),
            MachineSettings::ResourceConverter { modules, adjacent },
        ) => {
            (config.min_modules..=config.max_modules).contains(modules)
                && *adjacent <= config.max_adjacent
        }
        (
            Some(MachineKind::EndlessMiner(config)),
            MachineSettings::EndlessMiner { power_cores },
        ) => *power_cores <= config.power_core_slots,
        (
            Some(MachineKind::Reactor),
            MachineSettings::Reactor {
                utilization_percent,
            },
        ) => {
            utilization_percent.is_finite()
                && (0.0..=100.0).contains(utilization_percent)
                && (*utilization_percent / 10.0 - (*utilization_percent / 10.0).round()).abs()
                    < 0.0001
        }
        (Some(MachineKind::AssemblyLine(_)), MachineSettings::AssemblyLine { .. }) => true,
        (Some(MachineKind::FixedRate | MachineKind::Turbine { .. }), MachineSettings::Fixed) => {
            true
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        invalid(format!(
            "node {node_id} has settings incompatible with its machine"
        ))
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, PlanFileError> {
    Err(PlanFileError::InvalidPlan(message.into()))
}

impl From<&PlanNode> for SavedNode {
    fn from(node: &PlanNode) -> Self {
        Self {
            id: node.id,
            recipe_id: node.recipe_id.clone(),
            machine_id: node.machine_id.clone(),
            position: [node.position.x, node.position.y],
            pinned_primary_rate: node.pinned_primary_rate,
            settings: SavedMachineSettings::from(&node.settings),
        }
    }
}

impl From<&MachineSettings> for SavedMachineSettings {
    fn from(settings: &MachineSettings) -> Self {
        match settings {
            MachineSettings::Clock { percent } => Self::Clock { percent: *percent },
            MachineSettings::BlastFurnace(settings) => Self::BlastFurnace {
                towers: settings.towers,
                temperature: settings.temperature,
            },
            MachineSettings::ResourceConverter { modules, adjacent } => Self::ResourceConverter {
                modules: *modules,
                adjacent: *adjacent,
            },
            MachineSettings::EndlessMiner { power_cores } => Self::EndlessMiner {
                power_cores: *power_cores,
            },
            MachineSettings::Reactor {
                utilization_percent,
            } => Self::Reactor {
                utilization_percent: *utilization_percent,
            },
            MachineSettings::AssemblyLine { painted } => Self::AssemblyLine { painted: *painted },
            MachineSettings::Fixed => Self::Fixed,
        }
    }
}

impl From<SavedMachineSettings> for MachineSettings {
    fn from(settings: SavedMachineSettings) -> Self {
        match settings {
            SavedMachineSettings::Clock { percent } => Self::Clock { percent },
            SavedMachineSettings::BlastFurnace {
                towers,
                temperature,
            } => Self::BlastFurnace(BlastFurnaceSettings {
                towers,
                temperature,
            }),
            SavedMachineSettings::ResourceConverter { modules, adjacent } => {
                Self::ResourceConverter { modules, adjacent }
            }
            SavedMachineSettings::EndlessMiner { power_cores } => {
                Self::EndlessMiner { power_cores }
            }
            SavedMachineSettings::Reactor {
                utilization_percent,
            } => Self::Reactor {
                utilization_percent,
            },
            SavedMachineSettings::AssemblyLine { painted } => Self::AssemblyLine { painted },
            SavedMachineSettings::Fixed => Self::Fixed,
        }
    }
}

impl From<&Edge> for SavedEdge {
    fn from(edge: &Edge) -> Self {
        Self {
            from: SavedPort::from(&edge.from),
            to: SavedPort::from(&edge.to),
        }
    }
}

impl From<&PortRef> for SavedPort {
    fn from(port: &PortRef) -> Self {
        Self {
            node: port.node,
            side: match port.side {
                PortSide::Input => SavedPortSide::Input,
                PortSide::Output => SavedPortSide::Output,
            },
            slot: port.slot,
            item: port.item.clone(),
        }
    }
}

impl From<SavedPort> for PortRef {
    fn from(port: SavedPort) -> Self {
        Self {
            node: port.node,
            side: match port.side {
                SavedPortSide::Input => PortSide::Input,
                SavedPortSide::Output => PortSide::Output,
            },
            slot: port.slot,
            item: port.item,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Ingredient, MachineRecipeSelector, Recipe, RecipeKind};

    fn recipe(id: &str, input: Option<&str>, output: &str) -> Recipe {
        Recipe {
            id: id.to_owned(),
            name: id.to_owned(),
            inputs: input
                .map(|item| {
                    vec![Ingredient {
                        item: item.to_owned(),
                        amount: 1.0,
                    }]
                })
                .unwrap_or_default(),
            outputs: vec![Ingredient {
                item: output.to_owned(),
                amount: 1.0,
            }],
            time_seconds: 60.0,
            tags: vec!["test".to_owned()],
            category: String::new(),
            kind: RecipeKind::Crafting,
        }
    }

    fn test_data() -> GameData {
        GameData::from_test_parts(
            vec![
                recipe("source", None, "parts"),
                recipe("target", Some("parts"), "product"),
            ],
            vec![Machine {
                id: "machine".to_owned(),
                name: "Machine".to_owned(),
                recipe_selector: MachineRecipeSelector {
                    tags: vec!["test".to_owned()],
                    recipe_ids: Vec::new(),
                },
                speed: 1.0,
                power_kw: 10.0,
                kind: MachineKind::Crafting,
                required_resource_node: None,
            }],
        )
    }

    fn port(node: NodeId, side: PortSide, item: &str) -> PortRef {
        PortRef {
            node,
            side,
            slot: 0,
            item: item.to_owned(),
        }
    }

    #[test]
    fn plan_file_round_trip_restores_graph_view_and_id_allocator() {
        let data = test_data();
        let mut plan = Plan::default();
        let source = plan.add_recipe("source", Pos2::new(-12.5, 44.0), &data);
        let target = plan.add_recipe("target", Pos2::new(360.0, 120.0), &data);
        plan.nodes[0].pinned_primary_rate = Some(75.0);
        plan.nodes[0].settings = MachineSettings::Clock { percent: 125.0 };
        assert!(plan.connect(
            port(source, PortSide::Output, "parts"),
            port(target, PortSide::Input, "parts")
        ));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("factory.foundry-plan");
        save_plan(&path, &plan, Vec2::new(81.0, -17.0), 1.25).unwrap();
        let mut loaded = load_plan(&path, &data).unwrap();

        assert_eq!(loaded.plan.nodes.len(), 2);
        assert_eq!(loaded.plan.edges.len(), 1);
        assert_eq!(loaded.plan.nodes[0].position, Pos2::new(-12.5, 44.0));
        assert_eq!(loaded.plan.nodes[0].pinned_primary_rate, Some(75.0));
        assert_eq!(
            loaded.plan.nodes[0].settings,
            MachineSettings::Clock { percent: 125.0 }
        );
        assert_eq!(loaded.pan, Vec2::new(81.0, -17.0));
        assert_eq!(loaded.zoom, 1.25);
        assert_eq!(
            loaded.plan.evaluate(&data).nodes.len(),
            plan.evaluate(&data).nodes.len()
        );
        assert_eq!(
            loaded.plan.add_recipe("source", Pos2::ZERO, &data),
            target + 1
        );
    }

    #[test]
    fn every_machine_setting_variant_round_trips() {
        let settings = vec![
            MachineSettings::Clock { percent: 80.0 },
            MachineSettings::BlastFurnace(BlastFurnaceSettings {
                towers: 4,
                temperature: 1_900.0,
            }),
            MachineSettings::ResourceConverter {
                modules: 5,
                adjacent: 2,
            },
            MachineSettings::EndlessMiner { power_cores: 3 },
            MachineSettings::Reactor {
                utilization_percent: 70.0,
            },
            MachineSettings::AssemblyLine { painted: true },
            MachineSettings::Fixed,
        ];

        for original in settings {
            let json = serde_json::to_string(&SavedMachineSettings::from(&original)).unwrap();
            let saved: SavedMachineSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(MachineSettings::from(saved), original);
        }
    }

    #[test]
    fn unsupported_versions_are_reported_before_schema_details() {
        let json = r#"{"version":2,"future_format":true}"#;
        assert!(matches!(
            decode_plan(json, &test_data()),
            Err(PlanFileError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn missing_recipe_rejects_the_complete_file() {
        let data = test_data();
        let mut plan = Plan::default();
        plan.add_recipe("source", Pos2::ZERO, &data);
        let json = encode_plan(&plan, Vec2::ZERO, 1.0)
            .unwrap()
            .replace("\"source\"", "\"missing_recipe\"");

        let error = decode_plan(&json, &data).err().unwrap().to_string();
        assert!(error.contains("missing recipe 'missing_recipe'"));
    }

    #[test]
    fn malformed_edges_and_camera_values_are_rejected() {
        let data = test_data();
        let mut plan = Plan::default();
        let source = plan.add_recipe("source", Pos2::ZERO, &data);
        let target = plan.add_recipe("target", Pos2::new(10.0, 10.0), &data);
        plan.connect(
            port(source, PortSide::Output, "parts"),
            port(target, PortSide::Input, "parts"),
        );
        let mut file: PlanFileV1 =
            serde_json::from_str(&encode_plan(&plan, Vec2::ZERO, 1.0).unwrap()).unwrap();
        file.edges[0].to.side = SavedPortSide::Output;
        let invalid_edge = serde_json::to_string(&file).unwrap();
        assert!(decode_plan(&invalid_edge, &data).is_err());

        file.edges[0].to.side = SavedPortSide::Input;
        file.view.zoom = 4.0;
        let invalid_view = serde_json::to_string(&file).unwrap();
        assert!(decode_plan(&invalid_view, &data).is_err());
    }

    #[test]
    fn malformed_json_is_reported() {
        assert!(matches!(
            decode_plan("not json", &test_data()),
            Err(PlanFileError::InvalidJson(_))
        ));
    }
}

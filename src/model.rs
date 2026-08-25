use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui::Pos2;

use crate::data::{BlastFurnaceConfig, GameData, MachineKind, Recipe, RecipeKind};

pub type NodeId = u64;

const SOLVER_EPSILON: f64 = 1.0e-8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortSide {
    Input,
    Output,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PortRef {
    pub node: NodeId,
    pub side: PortSide,
    pub slot: usize,
    pub item: String,
}

#[derive(Clone, Debug)]
pub struct BlastFurnaceSettings {
    pub towers: u32,
    pub temperature: f32,
}

#[derive(Clone, Debug)]
pub struct PlanNode {
    pub id: NodeId,
    pub recipe_id: String,
    pub machine_id: Option<String>,
    pub position: Pos2,
    pub pinned_primary_rate: Option<f32>,
    pub clock_percent: f32,
    pub blast_furnace: Option<BlastFurnaceSettings>,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: PortRef,
    pub to: PortRef,
}

#[derive(Default)]
pub struct Plan {
    pub nodes: Vec<PlanNode>,
    pub edges: Vec<Edge>,
    next_id: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Balanced,
    Partial,
    Unresolved,
}

#[derive(Clone, Copy, Debug)]
pub struct NodeCalculation {
    pub primary_rate: f32,
    pub machines: f32,
    pub power_kw: f32,
}

#[derive(Default)]
pub struct PlanTotals {
    pub inputs: HashMap<String, f32>,
    pub outputs: HashMap<String, f32>,
    pub power_kw: f32,
    pub machine_count: f32,
    pub has_values: bool,
}

#[derive(Default)]
pub struct PlanEvaluation {
    pub nodes: HashMap<NodeId, NodeCalculation>,
    pub connection_states: HashMap<PortRef, ConnectionState>,
    pub totals: PlanTotals,
    pub unresolved_nodes: usize,
}

impl PlanEvaluation {
    pub fn node(&self, id: NodeId) -> Option<&NodeCalculation> {
        self.nodes.get(&id)
    }

    pub fn connection_state(&self, output: &PortRef) -> ConnectionState {
        self.connection_states
            .get(output)
            .copied()
            .unwrap_or(ConnectionState::Unresolved)
    }

    pub fn port_rate(&self, port: &PortRef, plan: &Plan, data: &GameData) -> Option<f32> {
        let node = plan.nodes.iter().find(|node| node.id == port.node)?;
        let calculation = self.node(node.id)?;
        let recipe = data.recipe(&node.recipe_id)?;
        match port.side {
            PortSide::Input => recipe.inputs.get(port.slot),
            PortSide::Output => recipe.outputs.get(port.slot),
        }?;
        Some(calculation.primary_rate * rate_ratio(node, recipe, port.side, port.slot, data))
    }

    pub fn port_connection_state(&self, port: &PortRef, plan: &Plan) -> ConnectionState {
        match port.side {
            PortSide::Output => self.connection_state(port),
            PortSide::Input => plan
                .edges
                .iter()
                .find(|edge| edge.to == *port)
                .map_or(ConnectionState::Unresolved, |edge| {
                    self.connection_state(&edge.from)
                }),
        }
    }
}

impl Plan {
    pub fn add_recipe(&mut self, recipe_id: &str, position: Pos2, data: &GameData) -> NodeId {
        self.next_id += 1;
        let machine_id = data.recipe(recipe_id).and_then(|recipe| {
            data.machine_options(recipe)
                .first()
                .map(|machine| machine.id.clone())
        });
        let blast_furnace = machine_id
            .as_deref()
            .and_then(|id| data.machine(id))
            .and_then(|machine| match &machine.kind {
                MachineKind::BlastFurnace(config) => Some(BlastFurnaceSettings {
                    towers: config.max_towers,
                    temperature: config.optimal_temperature,
                }),
                MachineKind::Crafting => None,
            });
        self.nodes.push(PlanNode {
            id: self.next_id,
            recipe_id: recipe_id.to_owned(),
            machine_id,
            position,
            pinned_primary_rate: None,
            clock_percent: 100.0,
            blast_furnace,
        });
        self.next_id
    }

    pub fn remove_node(&mut self, id: NodeId) {
        self.nodes.retain(|node| node.id != id);
        self.edges
            .retain(|edge| edge.from.node != id && edge.to.node != id);
    }

    pub fn connect(&mut self, first: PortRef, second: PortRef) -> bool {
        let (from, to) = match (first.side, second.side) {
            (PortSide::Output, PortSide::Input) => (first, second),
            (PortSide::Input, PortSide::Output) => (second, first),
            _ => return false,
        };
        if from.node == to.node || from.item != to.item {
            return false;
        }
        self.edges.retain(|edge| edge.to != to);
        if self
            .edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to)
        {
            return false;
        }
        self.edges.push(Edge { from, to });
        true
    }

    pub fn is_connected(&self, port: &PortRef) -> bool {
        self.edges
            .iter()
            .any(|edge| &edge.from == port || &edge.to == port)
    }

    pub fn evaluate(&self, data: &GameData) -> PlanEvaluation {
        let mut evaluation = PlanEvaluation::default();
        let node_indexes: HashMap<_, _> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();

        for component in self.connected_components() {
            let has_pin = component.iter().any(|id| {
                node_indexes
                    .get(id)
                    .and_then(|index| self.nodes[*index].pinned_primary_rate)
                    .is_some()
            });
            if !has_pin {
                continue;
            }

            let unknown_ids: Vec<_> = component
                .iter()
                .filter(|id| {
                    node_indexes
                        .get(id)
                        .is_some_and(|index| self.nodes[*index].pinned_primary_rate.is_none())
                })
                .copied()
                .collect();
            let unknown_columns: HashMap<_, _> = unknown_ids
                .iter()
                .enumerate()
                .map(|(column, id)| (*id, column))
                .collect();

            let mut equations = Vec::new();
            for output in self.connected_outputs_for(&component) {
                let Some((coefficients, rhs)) =
                    self.balance_equation(&output, &unknown_columns, &node_indexes, data)
                else {
                    continue;
                };
                if coefficients
                    .iter()
                    .any(|value| value.abs() > SOLVER_EPSILON)
                {
                    equations.push((coefficients, rhs));
                }
            }

            let solved = solve_unique_values(equations, unknown_ids.len());
            for id in &component {
                let Some(index) = node_indexes.get(id) else {
                    continue;
                };
                let node = &self.nodes[*index];
                let primary_rate = node.pinned_primary_rate.or_else(|| {
                    unknown_columns
                        .get(id)
                        .and_then(|column| solved.get(*column).copied().flatten())
                        .map(|value| value as f32)
                });
                let Some(primary_rate) =
                    primary_rate.filter(|rate| rate.is_finite() && *rate >= 0.0)
                else {
                    continue;
                };
                let Some(recipe) = data.recipe(&node.recipe_id) else {
                    continue;
                };
                let Some(machines) =
                    machine_count_for_primary_rate(node, recipe, primary_rate, data)
                else {
                    continue;
                };
                let power_kw = node
                    .machine_id
                    .as_deref()
                    .and_then(|id| data.machine(id))
                    .map_or(0.0, |machine| match machine.kind {
                        MachineKind::Crafting => {
                            machine.power_kw * machines * node.clock_percent.max(1.0) / 100.0
                        }
                        MachineKind::BlastFurnace(_) => machine.power_kw * machines,
                    });
                evaluation.nodes.insert(
                    *id,
                    NodeCalculation {
                        primary_rate,
                        machines,
                        power_kw,
                    },
                );
            }
        }

        evaluation.unresolved_nodes = self.nodes.len().saturating_sub(evaluation.nodes.len());
        evaluation.totals.has_values = !evaluation.nodes.is_empty();
        self.calculate_totals_and_connections(data, &mut evaluation);
        evaluation
    }

    fn connected_components(&self) -> Vec<Vec<NodeId>> {
        let mut neighbors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for node in &self.nodes {
            neighbors.entry(node.id).or_default();
        }
        for edge in &self.edges {
            neighbors
                .entry(edge.from.node)
                .or_default()
                .push(edge.to.node);
            neighbors
                .entry(edge.to.node)
                .or_default()
                .push(edge.from.node);
        }

        let mut visited = HashSet::new();
        let mut components = Vec::new();
        for node in &self.nodes {
            if !visited.insert(node.id) {
                continue;
            }
            let mut component = Vec::new();
            let mut queue = VecDeque::from([node.id]);
            while let Some(id) = queue.pop_front() {
                component.push(id);
                for neighbor in neighbors.get(&id).into_iter().flatten() {
                    if visited.insert(*neighbor) {
                        queue.push_back(*neighbor);
                    }
                }
            }
            components.push(component);
        }
        components
    }

    fn connected_outputs_for(&self, component: &[NodeId]) -> Vec<PortRef> {
        let ids: HashSet<_> = component.iter().copied().collect();
        let mut outputs = Vec::new();
        for edge in &self.edges {
            if ids.contains(&edge.from.node) && !outputs.contains(&edge.from) {
                outputs.push(edge.from.clone());
            }
        }
        outputs
    }

    fn balance_equation(
        &self,
        output: &PortRef,
        unknown_columns: &HashMap<NodeId, usize>,
        node_indexes: &HashMap<NodeId, usize>,
        data: &GameData,
    ) -> Option<(Vec<f64>, f64)> {
        let mut coefficients = vec![0.0; unknown_columns.len()];
        let mut constant = 0.0_f64;
        self.add_port_term(
            output,
            1.0,
            unknown_columns,
            node_indexes,
            data,
            &mut coefficients,
            &mut constant,
        )?;
        for edge in self.edges.iter().filter(|edge| edge.from == *output) {
            self.add_port_term(
                &edge.to,
                -1.0,
                unknown_columns,
                node_indexes,
                data,
                &mut coefficients,
                &mut constant,
            )?;
        }
        Some((coefficients, -constant))
    }

    #[allow(clippy::too_many_arguments)]
    fn add_port_term(
        &self,
        port: &PortRef,
        sign: f64,
        unknown_columns: &HashMap<NodeId, usize>,
        node_indexes: &HashMap<NodeId, usize>,
        data: &GameData,
        coefficients: &mut [f64],
        constant: &mut f64,
    ) -> Option<()> {
        let node = &self.nodes[*node_indexes.get(&port.node)?];
        let recipe = data.recipe(&node.recipe_id)?;
        match port.side {
            PortSide::Input => recipe.inputs.get(port.slot),
            PortSide::Output => recipe.outputs.get(port.slot),
        }?;
        let ratio = rate_ratio(node, recipe, port.side, port.slot, data) as f64 * sign;
        if let Some(rate) = node.pinned_primary_rate {
            *constant += ratio * rate as f64;
        } else if let Some(column) = unknown_columns.get(&node.id) {
            coefficients[*column] += ratio;
        }
        Some(())
    }

    fn calculate_totals_and_connections(&self, data: &GameData, evaluation: &mut PlanEvaluation) {
        for node in &self.nodes {
            let Some(calculation) = evaluation.node(node.id).copied() else {
                continue;
            };
            let Some(recipe) = data.recipe(&node.recipe_id) else {
                continue;
            };
            evaluation.totals.machine_count += calculation.machines;
            evaluation.totals.power_kw += calculation.power_kw;

            for (slot, ingredient) in recipe.inputs.iter().enumerate() {
                let port = PortRef {
                    node: node.id,
                    side: PortSide::Input,
                    slot,
                    item: ingredient.item.clone(),
                };
                if !self.is_connected(&port) {
                    *evaluation
                        .totals
                        .inputs
                        .entry(ingredient.item.clone())
                        .or_default() += calculation.primary_rate
                        * rate_ratio(node, recipe, PortSide::Input, slot, data);
                }
            }
            for (slot, ingredient) in recipe.outputs.iter().enumerate() {
                let port = PortRef {
                    node: node.id,
                    side: PortSide::Output,
                    slot,
                    item: ingredient.item.clone(),
                };
                if !self.is_connected(&port) {
                    *evaluation
                        .totals
                        .outputs
                        .entry(ingredient.item.clone())
                        .or_default() += calculation.primary_rate
                        * rate_ratio(node, recipe, PortSide::Output, slot, data);
                }
            }
        }

        let mut outputs = Vec::new();
        for edge in &self.edges {
            if !outputs.contains(&edge.from) {
                outputs.push(edge.from.clone());
            }
        }
        for output in outputs {
            let production = evaluation.port_rate(&output, self, data);
            let demands: Option<Vec<_>> = self
                .edges
                .iter()
                .filter(|edge| edge.from == output)
                .map(|edge| evaluation.port_rate(&edge.to, self, data))
                .collect();
            let Some((production, demands)) = production.zip(demands) else {
                evaluation
                    .connection_states
                    .insert(output, ConnectionState::Unresolved);
                continue;
            };
            let demand: f32 = demands.into_iter().sum();
            let difference = production - demand;
            let tolerance = 0.01_f32.max(production.abs().max(demand.abs()) * 1.0e-4);
            if difference.abs() <= tolerance {
                evaluation
                    .connection_states
                    .insert(output, ConnectionState::Balanced);
            } else {
                evaluation
                    .connection_states
                    .insert(output.clone(), ConnectionState::Partial);
                if difference > 0.0 {
                    *evaluation
                        .totals
                        .outputs
                        .entry(output.item.clone())
                        .or_default() += difference;
                } else {
                    *evaluation
                        .totals
                        .inputs
                        .entry(output.item.clone())
                        .or_default() += -difference;
                }
            }
        }
    }
}

pub fn primary_rate_for_machine_count(
    node: &PlanNode,
    recipe: &Recipe,
    machines: f32,
    data: &GameData,
) -> Option<f32> {
    Some(per_machine_port_rate(node, recipe, PortSide::Output, 0, data)? * machines)
}

fn machine_count_for_primary_rate(
    node: &PlanNode,
    recipe: &Recipe,
    primary_rate: f32,
    data: &GameData,
) -> Option<f32> {
    let one_machine_rate = primary_rate_for_machine_count(node, recipe, 1.0, data)?;
    (one_machine_rate > 0.0).then_some(primary_rate / one_machine_rate)
}

fn rate_ratio(
    node: &PlanNode,
    recipe: &Recipe,
    side: PortSide,
    slot: usize,
    data: &GameData,
) -> f32 {
    let Some(primary_rate) = per_machine_port_rate(node, recipe, PortSide::Output, 0, data) else {
        return 0.0;
    };
    per_machine_port_rate(node, recipe, side, slot, data).unwrap_or(0.0)
        / primary_rate.max(f32::EPSILON)
}

fn per_machine_port_rate(
    node: &PlanNode,
    recipe: &Recipe,
    side: PortSide,
    slot: usize,
    data: &GameData,
) -> Option<f32> {
    let ingredient = match side {
        PortSide::Input => recipe.inputs.get(slot),
        PortSide::Output => recipe.outputs.get(slot),
    }?;
    let machine = node.machine_id.as_deref().and_then(|id| data.machine(id));
    match (&recipe.kind, machine.map(|machine| &machine.kind)) {
        (
            RecipeKind::BlastFurnace {
                hot_air_input_slot, ..
            },
            Some(MachineKind::BlastFurnace(config)),
        ) if side == PortSide::Input && slot == *hot_air_input_slot => {
            Some(blast_furnace_hot_air_per_minute(node, config))
        }
        (RecipeKind::BlastFurnace { .. }, Some(MachineKind::BlastFurnace(config))) => {
            Some(recipe.base_rate(ingredient) * blast_furnace_operating_speed_for(node, config))
        }
        _ => {
            let speed = machine.map_or(1.0, |machine| machine.speed);
            Some(recipe.base_rate(ingredient) * speed * node.clock_percent.max(1.0) / 100.0)
        }
    }
}

pub fn blast_furnace_config<'a>(
    node: &PlanNode,
    data: &'a GameData,
) -> Option<&'a BlastFurnaceConfig> {
    match &data.machine(node.machine_id.as_deref()?)?.kind {
        MachineKind::BlastFurnace(config) => Some(config),
        MachineKind::Crafting => None,
    }
}

pub fn blast_furnace_operating_speed(node: &PlanNode, data: &GameData) -> Option<f32> {
    Some(blast_furnace_operating_speed_for(
        node,
        blast_furnace_config(node, data)?,
    ))
}

pub fn blast_furnace_hot_air_rate(node: &PlanNode, data: &GameData) -> Option<f32> {
    Some(blast_furnace_hot_air_per_minute(
        node,
        blast_furnace_config(node, data)?,
    ))
}

fn blast_furnace_operating_speed_for(node: &PlanNode, config: &BlastFurnaceConfig) -> f32 {
    let towers = furnace_towers(node, config);
    let optional_towers = towers.saturating_sub(config.min_towers) as f32;
    let max_speed = config.base_speed + optional_towers * config.tower_speed_increase;
    let temperature = node
        .blast_furnace
        .as_ref()
        .map_or(config.optimal_temperature, |settings| settings.temperature)
        .clamp(config.min_temperature, config.optimal_temperature);
    let range = config.optimal_temperature - config.min_temperature;
    if range <= f32::EPSILON {
        return max_speed.max(0.0);
    }
    let fraction = (temperature - config.min_temperature) / range;
    (config.speed_at_min_temperature + fraction * (max_speed - config.speed_at_min_temperature))
        .max(0.0)
}

fn blast_furnace_hot_air_per_minute(node: &PlanNode, config: &BlastFurnaceConfig) -> f32 {
    let towers = furnace_towers(node, config);
    let optional_towers = towers.saturating_sub(config.min_towers) as f32;
    config.base_hot_air_per_tick * 3_600.0 * (1.0 + optional_towers * config.tower_hot_air_increase)
}

fn furnace_towers(node: &PlanNode, config: &BlastFurnaceConfig) -> u32 {
    node.blast_furnace
        .as_ref()
        .map_or(config.max_towers, |settings| settings.towers)
        .clamp(config.min_towers, config.max_towers)
}

fn solve_unique_values(equations: Vec<(Vec<f64>, f64)>, variable_count: usize) -> Vec<Option<f64>> {
    if variable_count == 0 {
        return Vec::new();
    }
    let mut matrix: Vec<Vec<f64>> = equations
        .into_iter()
        .map(|(mut coefficients, rhs)| {
            coefficients.push(rhs);
            coefficients
        })
        .collect();
    if matrix.is_empty() {
        return vec![None; variable_count];
    }

    let mut pivot_rows = HashMap::new();
    let mut row = 0;
    for column in 0..variable_count {
        let Some(pivot) = (row..matrix.len()).max_by(|a, b| {
            matrix[*a][column]
                .abs()
                .total_cmp(&matrix[*b][column].abs())
        }) else {
            break;
        };
        if matrix[pivot][column].abs() <= SOLVER_EPSILON {
            continue;
        }
        matrix.swap(row, pivot);
        let divisor = matrix[row][column];
        for value in &mut matrix[row][column..=variable_count] {
            *value /= divisor;
        }
        let pivot_values = matrix[row].clone();
        for (other, other_row) in matrix.iter_mut().enumerate() {
            if other == row {
                continue;
            }
            let factor = other_row[column];
            if factor.abs() <= SOLVER_EPSILON {
                continue;
            }
            for (index, value) in other_row
                .iter_mut()
                .enumerate()
                .take(variable_count + 1)
                .skip(column)
            {
                *value -= factor * pivot_values[index];
            }
        }
        pivot_rows.insert(column, row);
        row += 1;
        if row == matrix.len() {
            break;
        }
    }

    if matrix.iter().any(|row| {
        row[..variable_count]
            .iter()
            .all(|value| value.abs() <= SOLVER_EPSILON)
            && row[variable_count].abs() > SOLVER_EPSILON
    }) {
        return vec![None; variable_count];
    }

    let free_columns: Vec<_> = (0..variable_count)
        .filter(|column| !pivot_rows.contains_key(column))
        .collect();
    let mut result = vec![None; variable_count];
    for (column, pivot_row) in pivot_rows {
        let uniquely_determined = free_columns
            .iter()
            .all(|free| matrix[pivot_row][*free].abs() <= SOLVER_EPSILON);
        if uniquely_determined {
            let value = matrix[pivot_row][variable_count];
            if value >= -SOLVER_EPSILON {
                result[column] = Some(value.max(0.0));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{
        BlastFurnaceConfig, Ingredient, Machine, MachineKind, RecipeKind, resolve_template_root,
    };

    fn recipe(
        id: &str,
        input_item: Option<&str>,
        input_rate: f32,
        output_item: &str,
        output_rate: f32,
    ) -> Recipe {
        Recipe {
            id: id.into(),
            name: id.into(),
            inputs: input_item
                .map(|item| {
                    vec![Ingredient {
                        item: item.into(),
                        amount: input_rate,
                    }]
                })
                .unwrap_or_default(),
            outputs: vec![Ingredient {
                item: output_item.into(),
                amount: output_rate,
            }],
            time_seconds: 60.0,
            tags: vec!["test".into()],
            category: String::new(),
            kind: RecipeKind::Crafting,
        }
    }

    fn test_data(recipes: Vec<Recipe>) -> GameData {
        GameData::from_test_parts(
            recipes,
            vec![Machine {
                id: "machine".into(),
                name: "Machine".into(),
                tags: vec!["test".into()],
                speed: 1.0,
                power_kw: 10.0,
                kind: MachineKind::Crafting,
            }],
        )
    }

    fn blast_furnace_data() -> GameData {
        GameData::from_test_parts(
            vec![Recipe {
                id: "blast_xf".into(),
                name: "Molten Xenoferrite".into(),
                inputs: vec![
                    Ingredient {
                        item: "ore".into(),
                        amount: 640.0,
                    },
                    Ingredient {
                        item: "coke".into(),
                        amount: 320.0,
                    },
                    Ingredient {
                        item: "rock".into(),
                        amount: 128.0,
                    },
                    Ingredient {
                        item: "hot_air".into(),
                        amount: 216_000.0,
                    },
                ],
                outputs: vec![
                    Ingredient {
                        item: "molten".into(),
                        amount: 5_760.0,
                    },
                    Ingredient {
                        item: "slag".into(),
                        amount: 640.0,
                    },
                    Ingredient {
                        item: "waste_gas".into(),
                        amount: 43_200.0,
                    },
                ],
                time_seconds: 60.0,
                tags: Vec::new(),
                category: "Blast Furnace".into(),
                kind: RecipeKind::BlastFurnace {
                    hot_air_input_slot: 3,
                    shutdown_slag: Some("slag".into()),
                },
            }],
            vec![Machine {
                id: "blast_furnace".into(),
                name: "Blast Furnace".into(),
                tags: Vec::new(),
                speed: 1.0,
                power_kw: 0.0,
                kind: MachineKind::BlastFurnace(BlastFurnaceConfig {
                    base_speed: 1.0,
                    output_multiplier: 4.0,
                    min_temperature: 1_500.0,
                    optimal_temperature: 2_000.0,
                    speed_at_min_temperature: 0.5,
                    hot_air_item: "hot_air".into(),
                    base_hot_air_per_tick: 60.0,
                    min_towers: 1,
                    max_towers: 5,
                    tower_speed_increase: 0.25,
                    tower_hot_air_increase: 0.125,
                }),
            }],
        )
    }

    fn port(node: NodeId, side: PortSide, item: &str) -> PortRef {
        PortRef {
            node,
            side,
            slot: 0,
            item: item.into(),
        }
    }

    fn pin(plan: &mut Plan, id: NodeId, primary_rate: f32) {
        plan.nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap()
            .pinned_primary_rate = Some(primary_rate);
    }

    fn evaluated_rate(
        evaluation: &PlanEvaluation,
        plan: &Plan,
        data: &GameData,
        node: NodeId,
        side: PortSide,
        slot: usize,
    ) -> f32 {
        let recipe = data.recipe("blast_xf").unwrap();
        let ingredient = match side {
            PortSide::Input => &recipe.inputs[slot],
            PortSide::Output => &recipe.outputs[slot],
        };
        evaluation
            .port_rate(
                &PortRef {
                    node,
                    side,
                    slot,
                    item: ingredient.item.clone(),
                },
                plan,
                data,
            )
            .unwrap()
    }

    #[test]
    fn blast_furnace_rates_follow_towers_and_temperature() {
        let data = blast_furnace_data();
        let mut plan = Plan::default();
        let node = plan.add_recipe("blast_xf", Pos2::ZERO, &data);
        let recipe = data.recipe("blast_xf").unwrap();

        let rate = primary_rate_for_machine_count(&plan.nodes[0], recipe, 1.0, &data).unwrap();
        assert!((rate - 11_520.0).abs() < 0.01);
        pin(&mut plan, node, rate);
        let evaluation = plan.evaluate(&data);
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Input, 0) - 1_280.0).abs()
                < 0.01
        );
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Input, 1) - 640.0).abs()
                < 0.01
        );
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Input, 2) - 256.0).abs()
                < 0.01
        );
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Input, 3) - 324_000.0).abs()
                < 0.01
        );
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Output, 1) - 1_280.0).abs()
                < 0.01
        );
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Output, 2) - 86_400.0).abs()
                < 0.01
        );

        plan.nodes[0].blast_furnace.as_mut().unwrap().towers = 1;
        let pinned_evaluation = plan.evaluate(&data);
        assert!((pinned_evaluation.node(node).unwrap().primary_rate - 11_520.0).abs() < 0.01);
        assert!((pinned_evaluation.node(node).unwrap().machines - 2.0).abs() < 0.01);
        let one_tower_rate =
            primary_rate_for_machine_count(&plan.nodes[0], recipe, 1.0, &data).unwrap();
        assert!((one_tower_rate - 5_760.0).abs() < 0.01);
        pin(&mut plan, node, one_tower_rate);
        let evaluation = plan.evaluate(&data);
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Input, 3) - 216_000.0).abs()
                < 0.01
        );
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Output, 2) - 43_200.0).abs()
                < 0.01
        );

        let settings = plan.nodes[0].blast_furnace.as_mut().unwrap();
        settings.towers = 5;
        settings.temperature = 1_500.0;
        let minimum_temperature_rate =
            primary_rate_for_machine_count(&plan.nodes[0], recipe, 1.0, &data).unwrap();
        assert!((minimum_temperature_rate - 2_880.0).abs() < 0.01);
        pin(&mut plan, node, minimum_temperature_rate);
        let evaluation = plan.evaluate(&data);
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Input, 3) - 324_000.0).abs()
                < 0.01
        );
        assert!(
            (evaluated_rate(&evaluation, &plan, &data, node, PortSide::Output, 2) - 21_600.0).abs()
                < 0.01
        );
    }

    #[test]
    fn unpinned_component_has_no_values() {
        let data = test_data(vec![recipe("product", None, 0.0, "product", 3.0)]);
        let mut plan = Plan::default();
        let node = plan.add_recipe("product", Pos2::ZERO, &data);
        let evaluation = plan.evaluate(&data);
        assert!(evaluation.node(node).is_none());
        assert!(!evaluation.totals.has_values);
    }

    #[test]
    fn pinned_demand_sizes_an_unpinned_producer() {
        let data = test_data(vec![
            recipe("air", Some("parts"), 300.0, "air", 3.0),
            recipe("parts", None, 0.0, "parts", 30.0),
        ]);
        let mut plan = Plan::default();
        let air = plan.add_recipe("air", Pos2::ZERO, &data);
        let parts = plan.add_recipe("parts", Pos2::ZERO, &data);
        pin(&mut plan, air, 3.0);
        assert!(plan.connect(
            port(parts, PortSide::Output, "parts"),
            port(air, PortSide::Input, "parts")
        ));

        let evaluation = plan.evaluate(&data);
        assert!((evaluation.node(parts).unwrap().machines - 10.0).abs() < 0.001);
        assert_eq!(
            evaluation.connection_state(&port(parts, PortSide::Output, "parts")),
            ConnectionState::Balanced
        );
        assert!(!evaluation.totals.inputs.contains_key("parts"));
    }

    #[test]
    fn mismatched_pins_report_deficit() {
        let data = test_data(vec![
            recipe("air", Some("parts"), 300.0, "air", 3.0),
            recipe("parts", None, 0.0, "parts", 30.0),
        ]);
        let mut plan = Plan::default();
        let air = plan.add_recipe("air", Pos2::ZERO, &data);
        let parts = plan.add_recipe("parts", Pos2::ZERO, &data);
        pin(&mut plan, air, 3.0);
        pin(&mut plan, parts, 30.0);
        plan.connect(
            port(parts, PortSide::Output, "parts"),
            port(air, PortSide::Input, "parts"),
        );

        let evaluation = plan.evaluate(&data);
        assert_eq!(
            evaluation.connection_state(&port(parts, PortSide::Output, "parts")),
            ConnectionState::Partial
        );
        assert!((evaluation.totals.inputs["parts"] - 270.0).abs() < 0.001);

        pin(&mut plan, parts, 600.0);
        let evaluation = plan.evaluate(&data);
        assert!((evaluation.totals.outputs["parts"] - 300.0).abs() < 0.001);
        assert!(!evaluation.totals.inputs.contains_key("parts"));
    }

    #[test]
    fn ambiguous_fan_out_remains_unresolved() {
        let data = test_data(vec![
            recipe("source", None, 0.0, "shared", 100.0),
            recipe("left", Some("shared"), 1.0, "left", 1.0),
            recipe("right", Some("shared"), 1.0, "right", 1.0),
        ]);
        let mut plan = Plan::default();
        let source = plan.add_recipe("source", Pos2::ZERO, &data);
        let left = plan.add_recipe("left", Pos2::ZERO, &data);
        let right = plan.add_recipe("right", Pos2::ZERO, &data);
        pin(&mut plan, source, 100.0);
        plan.connect(
            port(source, PortSide::Output, "shared"),
            port(left, PortSide::Input, "shared"),
        );
        plan.connect(
            port(source, PortSide::Output, "shared"),
            port(right, PortSide::Input, "shared"),
        );

        let evaluation = plan.evaluate(&data);
        assert!(evaluation.node(left).is_none());
        assert!(evaluation.node(right).is_none());
        assert_eq!(
            evaluation.connection_state(&port(source, PortSide::Output, "shared")),
            ConnectionState::Unresolved
        );
    }

    #[test]
    fn known_fan_out_uses_aggregate_demand() {
        let data = test_data(vec![
            recipe("source", None, 0.0, "shared", 100.0),
            recipe("left", Some("shared"), 1.0, "left", 1.0),
            recipe("right", Some("shared"), 1.0, "right", 1.0),
        ]);
        let mut plan = Plan::default();
        let source = plan.add_recipe("source", Pos2::ZERO, &data);
        let left = plan.add_recipe("left", Pos2::ZERO, &data);
        let right = plan.add_recipe("right", Pos2::ZERO, &data);
        pin(&mut plan, source, 100.0);
        pin(&mut plan, left, 40.0);
        plan.connect(
            port(source, PortSide::Output, "shared"),
            port(left, PortSide::Input, "shared"),
        );
        plan.connect(
            port(source, PortSide::Output, "shared"),
            port(right, PortSide::Input, "shared"),
        );

        let evaluation = plan.evaluate(&data);
        assert!((evaluation.node(right).unwrap().primary_rate - 60.0).abs() < 0.001);
        assert_eq!(
            evaluation.connection_state(&port(source, PortSide::Output, "shared")),
            ConnectionState::Balanced
        );
    }

    #[test]
    fn multi_hop_constraints_propagate() {
        let data = test_data(vec![
            recipe("final", Some("middle"), 40.0, "final", 10.0),
            recipe("middle", Some("raw"), 5.0, "middle", 20.0),
            recipe("raw", None, 0.0, "raw", 25.0),
        ]);
        let mut plan = Plan::default();
        let final_node = plan.add_recipe("final", Pos2::ZERO, &data);
        let middle = plan.add_recipe("middle", Pos2::ZERO, &data);
        let raw = plan.add_recipe("raw", Pos2::ZERO, &data);
        pin(&mut plan, final_node, 20.0);
        plan.connect(
            port(middle, PortSide::Output, "middle"),
            port(final_node, PortSide::Input, "middle"),
        );
        plan.connect(
            port(raw, PortSide::Output, "raw"),
            port(middle, PortSide::Input, "raw"),
        );

        let evaluation = plan.evaluate(&data);
        assert!((evaluation.node(middle).unwrap().primary_rate - 80.0).abs() < 0.001);
        assert!((evaluation.node(raw).unwrap().primary_rate - 20.0).abs() < 0.001);
        assert_eq!(evaluation.unresolved_nodes, 0);
    }

    #[test]
    fn conflicting_pins_leave_intermediate_node_unresolved() {
        let data = test_data(vec![
            recipe("source", None, 0.0, "shared", 10.0),
            recipe("middle", Some("shared"), 1.0, "middle", 1.0),
            recipe("target", Some("middle"), 20.0, "target", 1.0),
        ]);
        let mut plan = Plan::default();
        let source = plan.add_recipe("source", Pos2::ZERO, &data);
        let middle = plan.add_recipe("middle", Pos2::ZERO, &data);
        let target = plan.add_recipe("target", Pos2::ZERO, &data);
        pin(&mut plan, source, 10.0);
        pin(&mut plan, target, 1.0);
        plan.connect(
            port(source, PortSide::Output, "shared"),
            port(middle, PortSide::Input, "shared"),
        );
        plan.connect(
            port(middle, PortSide::Output, "middle"),
            port(target, PortSide::Input, "middle"),
        );

        let evaluation = plan.evaluate(&data);
        assert!(evaluation.node(middle).is_none());
        assert_eq!(
            evaluation.connection_state(&port(source, PortSide::Output, "shared")),
            ConnectionState::Unresolved
        );
        assert_eq!(
            evaluation.connection_state(&port(middle, PortSide::Output, "middle")),
            ConnectionState::Unresolved
        );
    }

    #[test]
    fn pinned_output_survives_machine_and_clock_changes() {
        let data = GameData::from_test_parts(
            vec![recipe("product", None, 0.0, "product", 5.0)],
            vec![Machine {
                id: "machine".into(),
                name: "Machine".into(),
                tags: vec!["test".into()],
                speed: 2.0,
                power_kw: 10.0,
                kind: MachineKind::Crafting,
            }],
        );
        let mut plan = Plan::default();
        let node = plan.add_recipe("product", Pos2::ZERO, &data);
        plan.nodes[0].clock_percent = 50.0;
        let rate = primary_rate_for_machine_count(
            &plan.nodes[0],
            data.recipe("product").unwrap(),
            3.0,
            &data,
        )
        .unwrap();
        pin(&mut plan, node, rate);
        assert!((plan.evaluate(&data).node(node).unwrap().machines - 3.0).abs() < 0.001);

        plan.nodes[0].clock_percent = 100.0;
        let calculation = plan.evaluate(&data).node(node).copied().unwrap();
        assert!((calculation.primary_rate - rate).abs() < 0.001);
        assert!((calculation.machines - 1.5).abs() < 0.001);
    }

    #[test]
    fn installed_air_intake_example_propagates_when_available() {
        let Ok(root) = resolve_template_root() else {
            return;
        };
        let data = GameData::load(&root).unwrap();
        let mut plan = Plan::default();
        let air = plan.add_recipe("_base_air_intake_base", Pos2::ZERO, &data);
        let parts = plan.add_recipe("_base_mic_ii", Pos2::ZERO, &data);
        let air_recipe = data.recipe("_base_air_intake_base").unwrap();
        let air_input = air_recipe
            .inputs
            .iter()
            .position(|ingredient| ingredient.item == "_base_mic_ii")
            .unwrap();
        let pinned_rate =
            primary_rate_for_machine_count(&plan.nodes[0], air_recipe, 1.0, &data).unwrap();
        pin(&mut plan, air, pinned_rate);
        assert!(plan.connect(
            port(parts, PortSide::Output, "_base_mic_ii"),
            PortRef {
                node: air,
                side: PortSide::Input,
                slot: air_input,
                item: "_base_mic_ii".into(),
            },
        ));

        let evaluation = plan.evaluate(&data);
        let parts = evaluation.node(parts).unwrap();
        assert!((parts.primary_rate - 300.0).abs() < 0.01);
        assert!((parts.machines - 10.0).abs() < 0.01);
    }

    #[test]
    fn installed_blast_furnace_connects_to_tier_three_plates_when_available() {
        let Ok(root) = resolve_template_root() else {
            return;
        };
        let data = GameData::load(&root).unwrap();
        let mut plan = Plan::default();
        let furnace = plan.add_recipe("_base_bfm_xf", Pos2::ZERO, &data);
        let plates = plan.add_recipe("_base_xf_plates_t3", Pos2::ZERO, &data);
        let plate_recipe = data.recipe("_base_xf_plates_t3").unwrap();
        let molten_input = plate_recipe
            .inputs
            .iter()
            .position(|ingredient| ingredient.item == "_base_molten_xf")
            .unwrap();
        let molten_output = data
            .recipe("_base_bfm_xf")
            .unwrap()
            .outputs
            .iter()
            .position(|ingredient| ingredient.item == "_base_molten_xf")
            .unwrap();
        let plate_rate =
            primary_rate_for_machine_count(&plan.nodes[1], plate_recipe, 1.0, &data).unwrap();
        pin(&mut plan, plates, plate_rate);
        assert!(plan.connect(
            PortRef {
                node: furnace,
                side: PortSide::Output,
                slot: molten_output,
                item: "_base_molten_xf".into(),
            },
            PortRef {
                node: plates,
                side: PortSide::Input,
                slot: molten_input,
                item: "_base_molten_xf".into(),
            },
        ));

        let evaluation = plan.evaluate(&data);
        assert!(evaluation.node(furnace).is_some());
        assert!(evaluation.node(plates).is_some());
        assert_eq!(evaluation.unresolved_nodes, 0);
        assert_eq!(
            evaluation.connection_state(&PortRef {
                node: furnace,
                side: PortSide::Output,
                slot: molten_output,
                item: "_base_molten_xf".into(),
            }),
            ConnectionState::Balanced
        );
    }
}

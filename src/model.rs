use std::collections::HashMap;

use eframe::egui::Pos2;

use crate::data::GameData;

pub type NodeId = u64;

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
pub struct PlanNode {
    pub id: NodeId,
    pub recipe_id: String,
    pub machine_id: Option<String>,
    pub position: Pos2,
    pub machines: f32,
    pub clock_percent: f32,
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

#[derive(Default)]
pub struct PlanTotals {
    pub inputs: HashMap<String, f32>,
    pub outputs: HashMap<String, f32>,
    pub power_kw: f32,
    pub machine_count: f32,
}

impl Plan {
    pub fn add_recipe(&mut self, recipe_id: &str, position: Pos2, data: &GameData) -> NodeId {
        self.next_id += 1;
        let machine_id = data.recipe(recipe_id).and_then(|recipe| {
            data.machine_options(recipe)
                .first()
                .map(|machine| machine.id.clone())
        });
        self.nodes.push(PlanNode {
            id: self.next_id,
            recipe_id: recipe_id.to_owned(),
            machine_id,
            position,
            machines: 1.0,
            clock_percent: 100.0,
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

    pub fn totals(&self, data: &GameData) -> PlanTotals {
        let mut totals = PlanTotals::default();
        for node in &self.nodes {
            let Some(recipe) = data.recipe(&node.recipe_id) else {
                continue;
            };
            let speed = node
                .machine_id
                .as_deref()
                .and_then(|id| data.machine(id))
                .map_or(1.0, |m| m.speed);
            let factor = speed * node.clock_percent.max(1.0) / 100.0 * node.machines.max(0.01);
            if let Some(machine) = node.machine_id.as_deref().and_then(|id| data.machine(id)) {
                totals.power_kw +=
                    machine.power_kw * node.machines.max(0.01) * node.clock_percent.max(1.0)
                        / 100.0;
            }
            totals.machine_count += node.machines.max(0.01);
            for (slot, ingredient) in recipe.inputs.iter().enumerate() {
                let port = PortRef {
                    node: node.id,
                    side: PortSide::Input,
                    slot,
                    item: ingredient.item.clone(),
                };
                if !self.is_connected(&port) {
                    *totals.inputs.entry(ingredient.item.clone()).or_default() +=
                        recipe.base_rate(ingredient) * factor;
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
                    *totals.outputs.entry(ingredient.item.clone()).or_default() +=
                        recipe.base_rate(ingredient) * factor;
                }
            }
        }
        totals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::resolve_template_root;

    #[test]
    fn connection_accounts_for_external_flow() {
        let Ok(path) = resolve_template_root() else {
            return;
        };
        let data = GameData::load(&path).unwrap();
        let mut plan = Plan::default();
        let smelter = plan.add_recipe("_base_xf_plates_t1", Pos2::ZERO, &data);
        let assembler = plan.add_recipe("_base_mic_i", Pos2::new(300.0, 0.0), &data);
        assert!(plan.connect(
            PortRef {
                node: smelter,
                side: PortSide::Output,
                slot: 0,
                item: "_base_xenoferrite_plates".into()
            },
            PortRef {
                node: assembler,
                side: PortSide::Input,
                slot: 0,
                item: "_base_xenoferrite_plates".into()
            },
        ));
        let totals = plan.totals(&data);
        assert!(!totals.inputs.contains_key("_base_xenoferrite_plates"));
        assert!(totals.inputs.contains_key("_base_rubble_xenoferrite"));
        assert!(totals.outputs.contains_key("_base_mic_i"));
        assert!(totals.power_kw > 0.0);
    }
}

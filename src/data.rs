use std::{
    collections::{BTreeSet, HashMap},
    env, fs,
    path::{Path, PathBuf},
};

use yaml_rust2::{Yaml, YamlLoader};

pub const TEMPLATE_ROOT_ENV: &str = "FOUNDRY_TEMPLATE_ROOT";

const TEMPLATE_COMPONENTS: [&str; 6] = [
    "steamapps",
    "common",
    "FOUNDRY",
    "foundry_Data",
    "StreamingAssets",
    "Templates",
];

pub fn resolve_template_root() -> Result<PathBuf, String> {
    let override_root = env::var_os(TEMPLATE_ROOT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    select_template_root(override_root, template_root_candidates(), Path::is_dir)
}

fn template_root_candidates() -> Vec<PathBuf> {
    let mut steam_roots = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if let Some(data_home) = env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            steam_roots.push(PathBuf::from(data_home).join("Steam"));
        }
        if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
            let home = PathBuf::from(home);
            steam_roots.push(home.join(".local/share/Steam"));
            steam_roots.push(home.join(".steam/steam"));
            steam_roots.push(home.join(".steam/debian-installation"));
            steam_roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
        steam_roots.push(PathBuf::from(home).join("Library/Application Support/Steam"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(program_files) =
            env::var_os("PROGRAMFILES(X86)").filter(|value| !value.is_empty())
        {
            steam_roots.push(PathBuf::from(program_files).join("Steam"));
        }
        if let Some(program_files) = env::var_os("PROGRAMFILES").filter(|value| !value.is_empty()) {
            steam_roots.push(PathBuf::from(program_files).join("Steam"));
        }
    }

    let mut candidates = Vec::new();
    for mut root in steam_roots {
        for component in TEMPLATE_COMPONENTS {
            root.push(component);
        }
        if !candidates.contains(&root) {
            candidates.push(root);
        }
    }
    candidates
}

fn select_template_root(
    override_root: Option<PathBuf>,
    candidates: Vec<PathBuf>,
    is_dir: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    if let Some(root) = override_root {
        return Ok(root);
    }
    if let Some(root) = candidates.iter().find(|path| is_dir(path)).cloned() {
        return Ok(root);
    }

    let checked = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let checked = if checked.is_empty() {
        "no standard locations were available on this platform".to_owned()
    } else {
        format!("checked: {checked}")
    };
    Err(format!(
        "Could not find the FOUNDRY template directory ({checked}). Set {TEMPLATE_ROOT_ENV} to the game's StreamingAssets/Templates directory."
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ingredient {
    pub item: String,
    pub amount: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateAnchor {
    Input(usize),
    Output(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecipeKind {
    Crafting,
    BlastFurnace {
        hot_air_input_slot: usize,
        shutdown_slag: Option<String>,
    },
    Direct {
        machine_id: String,
        anchor: RateAnchor,
        optional_input_slot: Option<usize>,
    },
}

#[derive(Clone, Debug)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub inputs: Vec<Ingredient>,
    pub outputs: Vec<Ingredient>,
    pub time_seconds: f32,
    pub tags: Vec<String>,
    pub category: String,
    pub kind: RecipeKind,
}

impl Recipe {
    pub fn base_rate(&self, ingredient: &Ingredient) -> f32 {
        ingredient.amount * 60.0 / self.time_seconds.max(0.001)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlastFurnaceConfig {
    pub base_speed: f32,
    pub output_multiplier: f32,
    pub min_temperature: f32,
    pub optimal_temperature: f32,
    pub speed_at_min_temperature: f32,
    pub hot_air_item: String,
    pub base_hot_air_per_tick: f32,
    pub min_towers: u32,
    pub max_towers: u32,
    pub tower_speed_increase: f32,
    pub tower_hot_air_increase: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResourceConverterConfig {
    pub min_modules: u32,
    pub max_modules: u32,
    pub ignored_modules: u32,
    pub speed_bonus_per_module: f32,
    pub max_adjacent: u32,
    pub power_decrease_per_adjacent: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EndlessMinerConfig {
    pub power_core_slots: u32,
    pub speed_increase_per_core: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyLineConfig {
    pub energy_per_product_kj: f32,
    pub painting_energy_per_product_kj: f32,
    pub painted_input_slot: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MachineKind {
    Crafting,
    BlastFurnace(BlastFurnaceConfig),
    ResourceConverter(ResourceConverterConfig),
    FixedRate,
    EndlessMiner(EndlessMinerConfig),
    Reactor,
    Turbine { generation_kw: f32 },
    AssemblyLine(AssemblyLineConfig),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineRecipeSelector {
    pub tags: Vec<String>,
    pub recipe_ids: Vec<String>,
}

impl MachineRecipeSelector {
    fn matches(&self, recipe: &Recipe, recipe_tags: &BTreeSet<&str>) -> bool {
        self.recipe_ids.iter().any(|id| id == &recipe.id)
            || self
                .tags
                .iter()
                .any(|tag| recipe_tags.contains(tag.as_str()))
    }

    fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.recipe_ids.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub recipe_selector: MachineRecipeSelector,
    pub speed: f32,
    pub power_kw: f32,
    pub kind: MachineKind,
    pub required_resource_node: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct ElementProfile {
    fuel_value_kj_per_l: f32,
    residual: Option<Ingredient>,
}

#[derive(Clone, Debug)]
struct AssemblyStarter {
    torso: String,
    object_id: String,
    output: String,
}

#[derive(Clone, Debug)]
struct AssemblyAction {
    power_kj: f32,
    required: Vec<Ingredient>,
    part: String,
    animation: String,
    left_arm: bool,
}

#[derive(Clone, Debug)]
struct AssemblyObject {
    id: String,
    name: String,
    attached_parts: usize,
    paint: Option<Ingredient>,
    painting_power_kj: f32,
}

#[derive(Clone, Debug, Default)]
pub struct GameData {
    pub recipes: Vec<Recipe>,
    pub machines: Vec<Machine>,
    pub item_names: HashMap<String, String>,
    pub tag_names: HashMap<String, String>,
    pub resource_node_names: HashMap<String, String>,
    element_profiles: HashMap<String, ElementProfile>,
    related_items: HashMap<String, String>,
    assembly_starters: Vec<AssemblyStarter>,
    recipe_index: HashMap<String, usize>,
    machine_index: HashMap<String, usize>,
    producers: HashMap<String, Vec<usize>>,
    consumers: HashMap<String, Vec<usize>>,
}

impl GameData {
    #[cfg(test)]
    pub(crate) fn from_test_parts(recipes: Vec<Recipe>, machines: Vec<Machine>) -> Self {
        let mut data = Self {
            recipes,
            machines,
            ..Self::default()
        };
        data.rebuild_indexes();
        data
    }

    pub fn load(root: &Path) -> Result<Self, String> {
        let crafting_dir = if root.ends_with("CraftingRecipe") {
            root.to_path_buf()
        } else {
            root.join("CraftingRecipe")
        };
        let templates = crafting_dir.parent().unwrap_or(root);

        if !crafting_dir.is_dir() {
            return Err(format!(
                "Recipe directory not found: {}",
                crafting_dir.display()
            ));
        }

        let mut data = Self::default();
        data.load_named_templates(&templates.join("ItemTemplate"), "ItemTemplate")?;
        data.load_named_templates(&templates.join("LiquidTemplate"), "LiquidTemplate")?;
        data.load_element_templates(&templates.join("ElementTemplate"))?;
        data.load_tags(&templates.join("CraftingTag"))?;
        data.load_assembly_starters(&templates.join("ItemTemplate"))?;
        data.load_resource_nodes(&templates.join("SpecialWorldObjectTemplate"))?;
        data.load_machines(&templates.join("BuildableObjectTemplate"))?;
        data.load_recipes(&crafting_dir)?;
        data.load_blast_furnace_modes(&templates.join("BlastFurnaceModeTemplate"))?;
        data.load_joined_extraction(templates)?;
        data.load_assembly_lines(templates)?;

        data.recipes.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        data.machines
            .sort_by_key(|machine| machine.name.to_lowercase());
        data.rebuild_indexes();
        Ok(data)
    }

    pub fn recipe(&self, id: &str) -> Option<&Recipe> {
        self.recipe_index.get(id).map(|&idx| &self.recipes[idx])
    }

    pub fn machine(&self, id: &str) -> Option<&Machine> {
        self.machine_index.get(id).map(|&idx| &self.machines[idx])
    }

    pub fn item_name(&self, id: &str) -> String {
        self.item_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| humanize_id(id))
    }

    pub fn machine_options<'a>(&'a self, recipe: &Recipe) -> Vec<&'a Machine> {
        let tags: BTreeSet<&str> = recipe.tags.iter().map(String::as_str).collect();
        let mut result: Vec<_> = self
            .machines
            .iter()
            .filter(|machine| match (&recipe.kind, &machine.kind) {
                (RecipeKind::BlastFurnace { .. }, MachineKind::BlastFurnace(_)) => true,
                (RecipeKind::Crafting, MachineKind::Crafting) => {
                    machine.recipe_selector.matches(recipe, &tags)
                }
                (RecipeKind::Direct { machine_id, .. }, _) => machine.id == *machine_id,
                _ => false,
            })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }

    pub fn recipes_producing(&self, item: &str) -> Vec<&Recipe> {
        self.producers
            .get(item)
            .into_iter()
            .flatten()
            .map(|&idx| &self.recipes[idx])
            .collect()
    }

    pub fn recipes_consuming(&self, item: &str) -> Vec<&Recipe> {
        self.consumers
            .get(item)
            .into_iter()
            .flatten()
            .map(|&idx| &self.recipes[idx])
            .collect()
    }

    fn rebuild_indexes(&mut self) {
        self.recipe_index.clear();
        self.machine_index.clear();
        self.producers.clear();
        self.consumers.clear();
        for (idx, recipe) in self.recipes.iter().enumerate() {
            self.recipe_index.insert(recipe.id.clone(), idx);
            for item in &recipe.inputs {
                self.consumers
                    .entry(item.item.clone())
                    .or_default()
                    .push(idx);
            }
            for item in &recipe.outputs {
                self.producers
                    .entry(item.item.clone())
                    .or_default()
                    .push(idx);
            }
        }
        for (idx, machine) in self.machines.iter().enumerate() {
            self.machine_index.insert(machine.id.clone(), idx);
        }
    }

    fn load_named_templates(&mut self, dir: &Path, root_key: &str) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }
        for path in yaml_files(dir)? {
            if let Ok(doc) = load_yaml(&path) {
                for (id, entry) in template_entries(&doc, root_key) {
                    let name = string_field(entry, "name")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| humanize_id(&id));
                    self.item_names.insert(id, name);
                }
            }
        }
        Ok(())
    }

    fn load_element_templates(&mut self, dir: &Path) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }
        for path in yaml_files(dir)? {
            let Ok(doc) = load_yaml(&path) else { continue };
            for (id, entry) in template_entries(&doc, "ElementTemplate") {
                let name = string_field(entry, "name")
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| humanize_id(&id));
                self.item_names.insert(id.clone(), name);
                let residual = non_empty_string_field(entry, "fuel_residualTemplate_identifier")
                    .zip(number_field(entry, "fuel_residualAmountPerL_str"))
                    .filter(|(_, amount)| *amount > 0.0)
                    .map(|(item, amount)| Ingredient { item, amount });
                self.element_profiles.insert(
                    id,
                    ElementProfile {
                        fuel_value_kj_per_l: number_field(entry, "fuel_fuelValueKJPerL_str")
                            .unwrap_or(0.0)
                            .max(0.0),
                        residual,
                    },
                );
            }
        }
        Ok(())
    }

    fn load_assembly_starters(&mut self, dir: &Path) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }
        for path in yaml_files(dir)? {
            let Ok(doc) = load_yaml(&path) else { continue };
            for (id, entry) in template_entries(&doc, "ItemTemplate") {
                let Some(object_id) = non_empty_string_field(entry, "alStarter_alotIdentifier")
                else {
                    continue;
                };
                let Some(output) =
                    non_empty_string_field(entry, "alStarter_sellItemTemplateIdentifier")
                else {
                    continue;
                };
                self.assembly_starters.push(AssemblyStarter {
                    torso: id,
                    object_id,
                    output,
                });
            }
        }
        Ok(())
    }

    fn load_resource_nodes(&mut self, dir: &Path) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }
        for path in yaml_files(dir)? {
            let Ok(doc) = load_yaml(&path) else { continue };
            for (id, entry) in template_entries(&doc, "SpecialWorldObjectTemplate") {
                if bool_field(entry, "isResourceNode") != Some(true) {
                    continue;
                }
                let resource_node_id =
                    non_empty_string_field(entry, "bot_identifier").unwrap_or(id);
                let name = string_field(entry, "name")
                    .filter(|name| !name.is_empty())
                    .or_else(|| self.item_names.get(&resource_node_id).cloned())
                    .unwrap_or_else(|| humanize_id(&resource_node_id));
                self.resource_node_names.insert(resource_node_id, name);
            }
        }
        Ok(())
    }

    fn load_tags(&mut self, dir: &Path) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }
        for path in yaml_files(dir)? {
            if let Ok(doc) = load_yaml(&path) {
                for (id, entry) in template_entries(&doc, "CraftingTag") {
                    let name = string_field(entry, "name")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| humanize_id(&id));
                    self.tag_names.insert(id, name);
                }
            }
        }
        Ok(())
    }

    fn load_machines(&mut self, dir: &Path) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }
        for path in yaml_files(dir)? {
            let Ok(doc) = load_yaml(&path) else { continue };
            for (id, entry) in template_entries(&doc, "BuildableObjectTemplate") {
                let buildable_type = string_field(entry, "type").unwrap_or_default();
                let is_blast_furnace = buildable_type == "BlastFurnace";
                let crafting_profile = crafting_machine_profile(entry, &buildable_type);
                let name = string_field(entry, "nameOverride")
                    .filter(|name| !name.is_empty())
                    .or_else(|| self.item_names.get(&id).cloned())
                    .unwrap_or_else(|| humanize_id(&id));
                let required_resource_node = bool_field(entry, "requiresResourceNode")
                    .filter(|required| *required)
                    .and_then(|_| non_empty_string_field(entry, "requiredResourceNodeIdentifier"))
                    .filter(|node| self.resource_node_names.contains_key(node));

                if let Some((recipe_selector, speed)) = crafting_profile {
                    self.machines.push(Machine {
                        id,
                        name,
                        recipe_selector,
                        speed,
                        power_kw: number_field(entry, "energyConsumptionKW_str")
                            .unwrap_or(0.0)
                            .max(0.0),
                        kind: MachineKind::Crafting,
                        required_resource_node,
                    });
                    continue;
                }

                if is_blast_furnace {
                    let tower_id = string_field(entry, "blastFurnace_towerModuleBotIdentifier")
                        .unwrap_or_default();
                    let (min_towers, max_towers) =
                        modular_limit(entry, &tower_id).unwrap_or((1, 1));
                    let kind = MachineKind::BlastFurnace(BlastFurnaceConfig {
                        base_speed: number_field(entry, "blastFurnace_speedModifier")
                            .unwrap_or(1.0)
                            .max(0.0),
                        // Mode output amounts are the steady-state rates used by the game UI;
                        // retain this template value as metadata without applying it twice.
                        output_multiplier: number_field(entry, "blastFurnace_outputMultiplier")
                            .unwrap_or(1.0)
                            .max(0.0),
                        min_temperature: number_field(entry, "blastFurnace_minRunningTemp")
                            .unwrap_or(0.0),
                        optimal_temperature: number_field(entry, "blastFurnace_optimalRunningTemp")
                            .unwrap_or(0.0),
                        speed_at_min_temperature: number_field(
                            entry,
                            "blastFurnace_speedPercentageAtMinRunningTemp",
                        )
                        .unwrap_or(0.0)
                        .max(0.0),
                        hot_air_item: string_field(entry, "blastFurnace_hotAirTemplateIdentifier")
                            .unwrap_or_default(),
                        base_hot_air_per_tick: number_field(
                            entry,
                            "blastFurnace_baseHotAirConsumptionPerTick",
                        )
                        .unwrap_or(0.0)
                        .max(0.0),
                        min_towers,
                        max_towers: max_towers.max(min_towers),
                        tower_speed_increase: number_field(
                            entry,
                            "blastFurnace_towerModule_speedIncrease",
                        )
                        .unwrap_or(0.0),
                        tower_hot_air_increase: number_field(
                            entry,
                            "blastFurnace_towerModule_hotAirConsumptionPercentIncrease",
                        )
                        .unwrap_or(0.0),
                    });
                    self.machines.push(Machine {
                        id,
                        name,
                        recipe_selector: MachineRecipeSelector::default(),
                        speed: 1.0,
                        power_kw: number_field(entry, "energyConsumptionKW_str")
                            .unwrap_or(0.0)
                            .max(0.0),
                        kind,
                        required_resource_node,
                    });
                    continue;
                }

                match buildable_type.as_str() {
                    "ResourceConverter" => {
                        let inputs = scaled_ingredient_list(
                            entry,
                            "resourceConverter_input_elemental",
                            60.0,
                        );
                        let outputs = scaled_ingredient_list(
                            entry,
                            "resourceConverter_output_elemental",
                            60.0,
                        );
                        if inputs.is_empty() && outputs.is_empty() {
                            continue;
                        }
                        let (module_id, speed_bonus_per_module, ignored_modules) =
                            resource_converter_module(entry).unwrap_or_default();
                        let (min_modules, max_modules) =
                            modular_limit(entry, &module_id).unwrap_or((0, 0));
                        let machine_id = id.clone();
                        self.machines.push(Machine {
                            id,
                            name: name.clone(),
                            recipe_selector: MachineRecipeSelector::default(),
                            speed: 1.0,
                            power_kw: number_field(
                                entry,
                                "resourceConverter_powerConsumption_kjPerSec",
                            )
                            .unwrap_or(0.0)
                            .max(0.0),
                            kind: MachineKind::ResourceConverter(ResourceConverterConfig {
                                min_modules,
                                max_modules: max_modules.max(min_modules),
                                ignored_modules,
                                speed_bonus_per_module,
                                max_adjacent: if bool_field(
                                    entry,
                                    "resourceConverter_hasAdjacencyBonus",
                                ) == Some(true)
                                {
                                    2
                                } else {
                                    0
                                },
                                power_decrease_per_adjacent: number_field(
                                    entry,
                                    "resourceConverter_powerDecreasePerAdjacentResourceConverter",
                                )
                                .unwrap_or(0.0)
                                .clamp(0.0, 1.0),
                            }),
                            required_resource_node,
                        });
                        self.push_direct_recipe(
                            format!("direct:{machine_id}"),
                            name,
                            inputs,
                            outputs,
                            "Fluid Processing",
                            machine_id,
                            RateAnchor::Output(0),
                            None,
                        );
                    }
                    "PipeIntake" => {
                        let Some(output_rate) = pipe_intake_output_rate_per_minute(entry) else {
                            continue;
                        };
                        let machine_id = id.clone();
                        self.machines.push(Machine {
                            id,
                            name: name.clone(),
                            recipe_selector: MachineRecipeSelector::default(),
                            speed: 1.0,
                            power_kw: 0.0,
                            kind: MachineKind::FixedRate,
                            required_resource_node: None,
                        });
                        self.push_direct_recipe(
                            format!("direct:{machine_id}"),
                            name,
                            Vec::new(),
                            vec![Ingredient {
                                item: "_base_water".to_owned(),
                                amount: output_rate,
                            }],
                            "Extraction",
                            machine_id,
                            RateAnchor::Output(0),
                            None,
                        );
                    }
                    "Boiler" => {
                        let Some(input) = direct_ingredient(
                            entry,
                            "boiler_elementTemplateIdentifier_source",
                            "boiler_consumptionPerSecond_str",
                            60.0,
                        ) else {
                            continue;
                        };
                        let Some(output) = direct_ingredient(
                            entry,
                            "boiler_elementTemplateIdentifier_output",
                            "boiler_outputPerSecond_str",
                            60.0,
                        ) else {
                            continue;
                        };
                        let machine_id = id.clone();
                        self.machines.push(Machine {
                            id,
                            name: name.clone(),
                            recipe_selector: MachineRecipeSelector::default(),
                            speed: 1.0,
                            power_kw: number_field(entry, "boiler_energyConsumption_kjPerS_str")
                                .unwrap_or(0.0)
                                .max(0.0),
                            kind: MachineKind::FixedRate,
                            required_resource_node,
                        });
                        self.push_direct_recipe(
                            format!("direct:{machine_id}"),
                            name,
                            vec![input],
                            vec![output],
                            "Fluid Processing",
                            machine_id,
                            RateAnchor::Output(0),
                            None,
                        );
                    }
                    "EndlessMiner" => {
                        let Some(resource_node) = required_resource_node.clone() else {
                            continue;
                        };
                        let Some(output_id) =
                            non_empty_string_field(entry, "endlessMiner_outputTemplateIdentifier")
                        else {
                            continue;
                        };
                        let Some(ticks) = number_field(entry, "endlessMiner_ticksPerItem")
                            .filter(|value| *value > 0.0)
                        else {
                            continue;
                        };
                        let machine_id = id.clone();
                        self.machines.push(Machine {
                            id,
                            name: name.clone(),
                            recipe_selector: MachineRecipeSelector::default(),
                            speed: 1.0,
                            power_kw: number_field(
                                entry,
                                "endlessMiner_powerDemandMining_kjPerSec_str",
                            )
                            .unwrap_or(0.0)
                            .max(0.0),
                            kind: MachineKind::EndlessMiner(EndlessMinerConfig {
                                power_core_slots: number_field(entry, "endlessMiner_powerCoreSlots")
                                    .unwrap_or(0.0)
                                    .max(0.0)
                                    as u32,
                                speed_increase_per_core: number_field(
                                    entry,
                                    "endlessMiner_speedIncreasePerPowerCore_str",
                                )
                                .unwrap_or(0.0)
                                .max(0.0),
                            }),
                            required_resource_node: Some(resource_node),
                        });
                        self.push_direct_recipe(
                            format!("direct:{machine_id}"),
                            name,
                            Vec::new(),
                            vec![Ingredient {
                                item: output_id,
                                amount: 3_600.0 / ticks,
                            }],
                            "Extraction",
                            machine_id,
                            RateAnchor::Output(0),
                            None,
                        );
                    }
                    "NPP_Reactor" => {
                        let Some(fuel) = direct_ingredient(
                            entry,
                            "nppReactor_input_elementalTemplate_identifier",
                            "nppReactor_maxInputPerTick_str",
                            3_600.0,
                        ) else {
                            continue;
                        };
                        let Some(output) = direct_ingredient(
                            entry,
                            "nppReactor_output_elementalTemplate_identifier",
                            "nppReactor_maxOutputPerTick_str",
                            3_600.0,
                        ) else {
                            continue;
                        };
                        let Some(depleted_id) = non_empty_string_field(
                            entry,
                            "nppReactor_steamGeneratorInput_elementalTemplate_identifier",
                        ) else {
                            continue;
                        };
                        let machine_id = id.clone();
                        self.machines.push(Machine {
                            id,
                            name: name.clone(),
                            recipe_selector: MachineRecipeSelector::default(),
                            speed: 1.0,
                            power_kw: 0.0,
                            kind: MachineKind::Reactor,
                            required_resource_node,
                        });
                        self.push_direct_recipe(
                            format!("direct:{machine_id}"),
                            name,
                            vec![
                                fuel,
                                Ingredient {
                                    item: depleted_id,
                                    amount: output.amount,
                                },
                            ],
                            vec![output],
                            "Nuclear Power",
                            machine_id,
                            RateAnchor::Output(0),
                            None,
                        );
                    }
                    "NPP_SteamTurbine" => {
                        let Some(fuel_id) =
                            non_empty_string_field(entry, "fme_lockedElementTemplateIdentifier")
                        else {
                            continue;
                        };
                        let generation_kw =
                            number_field(entry, "nppSteamTurbine_powerGenerationRate_kjPerSec_str")
                                .unwrap_or(0.0)
                                .max(0.0);
                        let efficiency = number_field(entry, "nppSteamTurbine_efficiency_str")
                            .unwrap_or(0.0)
                            .max(0.0);
                        let Some(profile) = self.element_profiles.get(&fuel_id) else {
                            continue;
                        };
                        let denominator = profile.fuel_value_kj_per_l * efficiency;
                        let Some(residual) = profile.residual.clone() else {
                            continue;
                        };
                        if generation_kw <= 0.0 || denominator <= 0.0 {
                            continue;
                        }
                        let input_rate = generation_kw / denominator * 60.0;
                        let machine_id = id.clone();
                        self.machines.push(Machine {
                            id,
                            name: name.clone(),
                            recipe_selector: MachineRecipeSelector::default(),
                            speed: 1.0,
                            power_kw: 0.0,
                            kind: MachineKind::Turbine { generation_kw },
                            required_resource_node,
                        });
                        self.push_direct_recipe(
                            format!("direct:{machine_id}"),
                            name,
                            vec![Ingredient {
                                item: fuel_id,
                                amount: input_rate,
                            }],
                            vec![Ingredient {
                                item: residual.item,
                                amount: input_rate * residual.amount,
                            }],
                            "Nuclear Power",
                            machine_id,
                            RateAnchor::Output(0),
                            None,
                        );
                    }
                    "NPP_CoolingTower" => {
                        let Some(input) = direct_ingredient(
                            entry,
                            "nppCoolingTower_inputTurbine_identifier",
                            "nppCoolingTower_inputTurbine_maxInputPerTick_str",
                            3_600.0,
                        ) else {
                            continue;
                        };
                        let machine_id = id.clone();
                        self.machines.push(Machine {
                            id,
                            name: name.clone(),
                            recipe_selector: MachineRecipeSelector::default(),
                            speed: 1.0,
                            power_kw: 0.0,
                            kind: MachineKind::FixedRate,
                            required_resource_node,
                        });
                        self.push_direct_recipe(
                            format!("direct:{machine_id}"),
                            name,
                            vec![input],
                            Vec::new(),
                            "Nuclear Power",
                            machine_id,
                            RateAnchor::Input(0),
                            None,
                        );
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn push_direct_recipe(
        &mut self,
        id: String,
        name: String,
        inputs: Vec<Ingredient>,
        outputs: Vec<Ingredient>,
        category: &str,
        machine_id: String,
        anchor: RateAnchor,
        optional_input_slot: Option<usize>,
    ) {
        if (inputs.is_empty() && outputs.is_empty())
            || inputs.iter().chain(&outputs).any(|ingredient| {
                ingredient.item.is_empty()
                    || !ingredient.amount.is_finite()
                    || ingredient.amount <= 0.0
            })
        {
            return;
        }
        self.recipes.push(Recipe {
            id,
            name,
            inputs,
            outputs,
            time_seconds: 60.0,
            tags: Vec::new(),
            category: category.to_owned(),
            kind: RecipeKind::Direct {
                machine_id,
                anchor,
                optional_input_slot,
            },
        });
    }

    fn load_recipes(&mut self, dir: &Path) -> Result<(), String> {
        let mut failures = 0usize;
        for path in yaml_files(dir)? {
            let doc = match load_yaml(&path) {
                Ok(doc) => doc,
                Err(_) => {
                    failures += 1;
                    continue;
                }
            };
            for (id, entry) in template_entries(&doc, "CraftingRecipe") {
                let Some(name) = string_field(entry, "name") else {
                    continue;
                };
                if name.trim().is_empty() {
                    continue;
                }
                let inputs: Vec<Ingredient> = ingredient_list(entry, "input_data")
                    .into_iter()
                    .chain(ingredient_list(entry, "inputElemental_data"))
                    .collect();
                let outputs: Vec<Ingredient> = ingredient_list(entry, "output_data")
                    .into_iter()
                    .chain(ingredient_list(entry, "outputElemental_data"))
                    .collect();
                if let Some(related) =
                    non_empty_string_field(entry, "relatedItemTemplateIdentifier")
                {
                    for output in &outputs {
                        self.related_items
                            .insert(output.item.clone(), related.clone());
                    }
                }
                if inputs.is_empty() && outputs.is_empty() {
                    continue;
                }
                let time_seconds = number_field(entry, "timeMs").unwrap_or(1000.0) / 1000.0;
                self.recipes.push(Recipe {
                    id,
                    name,
                    inputs,
                    outputs,
                    time_seconds: time_seconds.max(0.001),
                    tags: string_list(entry, "tags"),
                    category: string_field(entry, "category_identifier").unwrap_or_default(),
                    kind: RecipeKind::Crafting,
                });
            }
        }
        if self.recipes.is_empty() {
            return Err(format!(
                "No recipes could be read from {} ({failures} invalid files)",
                dir.display()
            ));
        }
        Ok(())
    }

    fn load_blast_furnace_modes(&mut self, dir: &Path) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }
        let Some(config) = self
            .machines
            .iter()
            .find_map(|machine| match &machine.kind {
                MachineKind::BlastFurnace(config) => Some(config.clone()),
                _ => None,
            })
        else {
            return Ok(());
        };

        for path in yaml_files(dir)? {
            let Ok(doc) = load_yaml(&path) else { continue };
            for (id, entry) in template_entries(&doc, "BlastFurnaceModeTemplate") {
                let mut inputs = ingredient_list(entry, "input_data");
                let hot_air_input_slot = inputs.len();
                if !config.hot_air_item.is_empty() && config.base_hot_air_per_tick > 0.0 {
                    inputs.push(Ingredient {
                        item: config.hot_air_item.clone(),
                        amount: config.base_hot_air_per_tick * 3_600.0,
                    });
                }
                let mut outputs = ingredient_list(entry, "output_data_elemental");
                if let Some(waste_gas) = ingredient(entry, "waste_gas_data") {
                    outputs.push(Ingredient {
                        item: waste_gas.item,
                        amount: waste_gas.amount * 3_600.0,
                    });
                }
                if inputs.is_empty() && outputs.is_empty() {
                    continue;
                }
                let name = outputs
                    .first()
                    .map(|output| self.item_name(&output.item))
                    .or_else(|| string_field(entry, "name"))
                    .unwrap_or_else(|| humanize_id(&id));
                self.recipes.push(Recipe {
                    id,
                    name,
                    inputs,
                    outputs,
                    time_seconds: 60.0,
                    tags: Vec::new(),
                    category: "Blast Furnace".to_owned(),
                    kind: RecipeKind::BlastFurnace {
                        hot_air_input_slot,
                        shutdown_slag: string_field(entry, "slagTemplateIdentifierForShutdown")
                            .filter(|slag| !slag.is_empty()),
                    },
                });
            }
        }
        Ok(())
    }

    fn load_joined_extraction(&mut self, templates: &Path) -> Result<(), String> {
        let buildables = templates.join("BuildableObjectTemplate");
        self.load_pumpjack_processes(&buildables, &templates.join("ReservoirTemplate"))?;
        self.load_ore_vein_processes(
            &buildables,
            &templates.join("OreVeinTemplate"),
            &templates.join("TerrainBlockType"),
        )?;
        Ok(())
    }

    fn load_pumpjack_processes(
        &mut self,
        buildable_dir: &Path,
        reservoir_dir: &Path,
    ) -> Result<(), String> {
        let mut reservoirs = Vec::new();
        if reservoir_dir.is_dir() {
            for path in yaml_files(reservoir_dir)? {
                let Ok(doc) = load_yaml(&path) else { continue };
                for (id, entry) in template_entries(&doc, "ReservoirTemplate") {
                    if let Some(element) = non_empty_string_field(entry, "elementIdentifier") {
                        reservoirs.push((id, element));
                    }
                }
            }
        }
        if reservoirs.is_empty() || !buildable_dir.is_dir() {
            return Ok(());
        }
        for path in yaml_files(buildable_dir)? {
            let Ok(doc) = load_yaml(&path) else { continue };
            for (id, entry) in template_entries(&doc, "BuildableObjectTemplate") {
                if string_field(entry, "type").as_deref() != Some("Pumpjack") {
                    continue;
                }
                let Some(per_second) =
                    number_field(entry, "pumpjack_amountPerSec_str").filter(|rate| *rate > 0.0)
                else {
                    continue;
                };
                let name = buildable_name(self, &id, entry);
                self.machines.push(Machine {
                    id: id.clone(),
                    name: name.clone(),
                    recipe_selector: MachineRecipeSelector::default(),
                    speed: 1.0,
                    power_kw: number_field(entry, "energyConsumptionKW_str")
                        .unwrap_or(0.0)
                        .max(0.0),
                    kind: MachineKind::FixedRate,
                    required_resource_node: None,
                });
                for (reservoir_id, element) in &reservoirs {
                    let output_name = self.item_name(element);
                    self.push_direct_recipe(
                        format!("direct:{id}:{reservoir_id}"),
                        format!("Extract {output_name}"),
                        Vec::new(),
                        vec![Ingredient {
                            item: element.clone(),
                            amount: per_second * 60.0,
                        }],
                        "Extraction",
                        id.clone(),
                        RateAnchor::Output(0),
                        None,
                    );
                }
            }
        }
        Ok(())
    }

    fn load_ore_vein_processes(
        &mut self,
        buildable_dir: &Path,
        vein_dir: &Path,
        terrain_dir: &Path,
    ) -> Result<(), String> {
        let mut terrain_outputs = HashMap::new();
        if terrain_dir.is_dir() {
            for path in yaml_files(terrain_dir)? {
                let Ok(doc) = load_yaml(&path) else { continue };
                for (id, entry) in template_entries(&doc, "TerrainBlockType") {
                    let output =
                        non_empty_string_field(entry, "oreVeinMineable_yieldItem_identifier")
                            .or_else(|| non_empty_string_field(entry, "rmd_miningYield"));
                    if let Some(output) = output {
                        terrain_outputs.insert(id, output);
                    }
                }
            }
        }
        let mut veins = Vec::new();
        if vein_dir.is_dir() {
            for path in yaml_files(vein_dir)? {
                let Ok(doc) = load_yaml(&path) else { continue };
                for (id, entry) in template_entries(&doc, "OreVeinTemplate") {
                    let Some(block) = non_empty_string_field(entry, "mineableBlockType_identifier")
                    else {
                        continue;
                    };
                    let Some(output) = terrain_outputs.get(&block).cloned() else {
                        continue;
                    };
                    let Some(fluid) = non_empty_string_field(entry, "miningFluid_identifier")
                    else {
                        continue;
                    };
                    let Some(fluid_rate) =
                        number_field(entry, "requiredMiningFluid_literPerMinutePerMiner_str")
                            .filter(|rate| *rate > 0.0)
                    else {
                        continue;
                    };
                    veins.push((id, output, fluid, fluid_rate));
                }
            }
        }
        if veins.is_empty() || !buildable_dir.is_dir() {
            return Ok(());
        }

        let mut fracking_power_per_miner = 0.0;
        let mut miner: Option<(String, String, f32, f32)> = None;
        for path in yaml_files(buildable_dir)? {
            let Ok(doc) = load_yaml(&path) else { continue };
            for (id, entry) in template_entries(&doc, "BuildableObjectTemplate") {
                match string_field(entry, "type").as_deref() {
                    Some("FrackingTower") => {
                        let power = number_field(entry, "energyConsumptionKW_str")
                            .unwrap_or(0.0)
                            .max(0.0);
                        let throughput = number_field(
                            entry,
                            "frackingTower_fluidThroughputPerTowerPerSecond_str",
                        )
                        .unwrap_or(0.0)
                        .max(0.0)
                            * 60.0;
                        let representative_fluid = veins[0].3;
                        if throughput > 0.0 && representative_fluid > 0.0 {
                            fracking_power_per_miner = power / (throughput / representative_fluid);
                        }
                    }
                    Some("OreVeinMiner") => {
                        let ticks = number_field(entry, "oreVeinMiner_ticksPerOre_str")
                            .unwrap_or(0.0)
                            .max(0.0);
                        let power =
                            number_field(entry, "oreVeinMiner_powerConsumptionBase_kjPerSec")
                                .unwrap_or(0.0)
                                .max(0.0)
                                + number_field(
                                    entry,
                                    "oreVeinMiner_powerConsumptionMining_kjPerSec",
                                )
                                .unwrap_or(0.0)
                                .max(0.0);
                        miner = Some((id.clone(), buildable_name(self, &id, entry), ticks, power));
                    }
                    _ => {}
                }
            }
        }
        let Some((machine_id, machine_name, ticks_per_ore, miner_power)) = miner else {
            return Ok(());
        };
        if ticks_per_ore <= 0.0 {
            return Ok(());
        }
        self.machines.push(Machine {
            id: machine_id.clone(),
            name: machine_name,
            recipe_selector: MachineRecipeSelector::default(),
            speed: 1.0,
            power_kw: miner_power + fracking_power_per_miner,
            kind: MachineKind::FixedRate,
            required_resource_node: None,
        });
        let output_rate = 3_600.0 / ticks_per_ore;
        for (vein_id, output, fluid, fluid_rate) in veins {
            let output_name = self.item_name(&output);
            self.push_direct_recipe(
                format!("direct:{machine_id}:{vein_id}"),
                format!("Fracked {output_name}"),
                vec![Ingredient {
                    item: fluid,
                    amount: fluid_rate,
                }],
                vec![Ingredient {
                    item: output,
                    amount: output_rate,
                }],
                "Extraction",
                machine_id.clone(),
                RateAnchor::Output(0),
                None,
            );
        }
        Ok(())
    }

    fn load_assembly_lines(&mut self, templates: &Path) -> Result<(), String> {
        const PRODUCTS_PER_MINUTE: f32 = 32.0;
        let actions = load_assembly_actions(&templates.join("AssemblyLineProducerActionTemplate"))?;
        let objects = load_assembly_objects(&templates.join("AssemblyLineObjectTemplate"))?;
        if actions.is_empty() || objects.is_empty() {
            return Ok(());
        }
        let starter_power = assembly_starter_power(&templates.join("BuildableObjectTemplate"))?;

        for object in objects {
            let Some(starter) = self
                .assembly_starters
                .iter()
                .find(|starter| starter.object_id == object.id)
                .cloned()
            else {
                continue;
            };
            let attach_actions: Vec<_> = actions
                .iter()
                .filter(|action| {
                    !action.part.is_empty()
                        && action.required.iter().any(|ingredient| {
                            self.related_items.get(&ingredient.item) == Some(&starter.output)
                        })
                })
                .collect();
            if attach_actions.len() != object.attached_parts {
                continue;
            }

            let mut inputs = vec![Ingredient {
                item: starter.torso.clone(),
                amount: 1.0,
            }];
            let mut energy_per_product = starter_power;
            let mut complete = true;
            for attach in attach_actions {
                merge_ingredients(&mut inputs, &attach.required);
                energy_per_product += attach.power_kj;
                let weld_animation = attach.animation.replacen("attach_", "weld_", 1);
                let welds: Vec<_> = actions
                    .iter()
                    .filter(|action| {
                        action.part.is_empty()
                            && action.animation == weld_animation
                            && action.left_arm == attach.left_arm
                    })
                    .collect();
                let Some(first) = welds.first() else {
                    complete = false;
                    break;
                };
                if welds.iter().any(|candidate| {
                    (candidate.power_kj - first.power_kj).abs() > f32::EPSILON
                        || candidate.required != first.required
                }) {
                    complete = false;
                    break;
                }
                merge_ingredients(&mut inputs, &first.required);
                energy_per_product += first.power_kj;
            }
            if !complete {
                continue;
            }
            let painted_input_slot = object.paint.as_ref().map(|paint| {
                let slot = inputs.len();
                inputs.push(paint.clone());
                slot
            });
            let machine_id = format!("assembly-line:{}", object.id);
            self.machines.push(Machine {
                id: machine_id.clone(),
                name: format!("{} Assembly Line", object.name),
                recipe_selector: MachineRecipeSelector::default(),
                speed: 1.0,
                power_kw: 0.0,
                kind: MachineKind::AssemblyLine(AssemblyLineConfig {
                    energy_per_product_kj: energy_per_product.max(0.0),
                    painting_energy_per_product_kj: object.painting_power_kj,
                    painted_input_slot,
                }),
                required_resource_node: None,
            });
            let optional_slot = painted_input_slot;
            let mut outputs = vec![Ingredient {
                item: starter.output,
                amount: 1.0,
            }];
            // Express assembly material quantities per product; the explicit cycle time gives
            // one complete line the documented 32-products/minute capacity.
            let time_seconds = 60.0 / PRODUCTS_PER_MINUTE;
            self.recipes.push(Recipe {
                id: format!("assembly:{}", object.id),
                name: object.name,
                inputs: std::mem::take(&mut inputs),
                outputs: std::mem::take(&mut outputs),
                time_seconds,
                tags: Vec::new(),
                category: "Assembly Line".to_owned(),
                kind: RecipeKind::Direct {
                    machine_id,
                    anchor: RateAnchor::Output(0),
                    optional_input_slot: optional_slot,
                },
            });
        }
        Ok(())
    }
}

fn buildable_name(data: &GameData, id: &str, entry: &Yaml) -> String {
    string_field(entry, "nameOverride")
        .filter(|name| !name.is_empty())
        .or_else(|| data.item_names.get(id).cloned())
        .unwrap_or_else(|| humanize_id(id))
}

fn direct_ingredient(
    entry: &Yaml,
    item_key: &str,
    rate_key: &str,
    scale: f32,
) -> Option<Ingredient> {
    let ingredient = Ingredient {
        item: non_empty_string_field(entry, item_key)?,
        amount: number_field(entry, rate_key)? * scale,
    };
    (ingredient.amount.is_finite() && ingredient.amount > 0.0).then_some(ingredient)
}

fn scaled_ingredient_list(entry: &Yaml, key: &str, scale: f32) -> Vec<Ingredient> {
    ingredient_list(entry, key)
        .into_iter()
        .map(|mut ingredient| {
            ingredient.amount *= scale;
            ingredient
        })
        .filter(|ingredient| ingredient.amount.is_finite() && ingredient.amount > 0.0)
        .collect()
}

fn pipe_intake_output_rate_per_minute(entry: &Yaml) -> Option<f32> {
    let per_second = value(entry, "fbm_ioFluidBoxes")?
        .as_vec()?
        .iter()
        .find(|fluid_box| bool_field(fluid_box, "isInput") == Some(false))
        .and_then(|fluid_box| number_field(fluid_box, "transferRatePerSecond_liter"))?;
    let per_minute = per_second * 60.0;
    (per_minute.is_finite() && per_minute > 0.0).then_some(per_minute)
}

fn resource_converter_module(entry: &Yaml) -> Option<(String, f32, u32)> {
    let module = value(entry, "resourceConverter_speedBonusModules")?
        .as_vec()?
        .first()?;
    Some((
        non_empty_string_field(module, "bot_identifier")?,
        number_field(module, "speedBonus").unwrap_or(0.0).max(0.0),
        number_field(module, "numberOfIgnoredModules")
            .unwrap_or(0.0)
            .max(0.0) as u32,
    ))
}

fn load_assembly_actions(dir: &Path) -> Result<Vec<AssemblyAction>, String> {
    let mut actions = Vec::new();
    if !dir.is_dir() {
        return Ok(actions);
    }
    for path in yaml_files(dir)? {
        let Ok(doc) = load_yaml(&path) else { continue };
        for (_, entry) in template_entries(&doc, "AssemblyLineProducerActionTemplate") {
            let power_kj = number_field(entry, "powerCost_kj_str")
                .unwrap_or(0.0)
                .max(0.0);
            let mut required = ingredient_list(entry, "requiredItems");
            required.extend(ingredient_list(entry, "requiredElements"));
            actions.push(AssemblyAction {
                power_kj,
                required,
                part: string_field(entry, "partIdentifierToAttach").unwrap_or_default(),
                animation: string_field(entry, "animId").unwrap_or_default(),
                left_arm: bool_field(entry, "animateLeftProducerArm").unwrap_or(false),
            });
        }
    }
    Ok(actions)
}

fn load_assembly_objects(dir: &Path) -> Result<Vec<AssemblyObject>, String> {
    let mut objects = Vec::new();
    if !dir.is_dir() {
        return Ok(objects);
    }
    for path in yaml_files(dir)? {
        let Ok(doc) = load_yaml(&path) else { continue };
        for (id, entry) in template_entries(&doc, "AssemblyLineObjectTemplate") {
            let Some(parts) = value(entry, "objectParts").and_then(Yaml::as_vec) else {
                continue;
            };
            let attached_parts = parts
                .iter()
                .filter(|part| bool_field(part, "isInitialPart") != Some(true))
                .count();
            if attached_parts == 0 {
                continue;
            }
            let mut painting_power_kj = 0.0;
            let mut paint_item: Option<String> = None;
            let mut paint_amount = 0.0;
            let mut valid_paint = true;
            for part in parts {
                painting_power_kj += number_field(part, "requiredPowerForPainting_kj_str")
                    .unwrap_or(0.0)
                    .max(0.0);
                let variants = value(part, "colorVariants")
                    .and_then(Yaml::as_vec)
                    .into_iter()
                    .flatten();
                let colored: Vec<_> = variants
                    .filter_map(|variant| {
                        let item =
                            non_empty_string_field(variant, "paintElementTemplate_identifier")?;
                        let amount = number_field(variant, "amountLiquidRequired_l_str")?;
                        (amount > 0.0).then_some((item, amount))
                    })
                    .collect();
                if let Some((first_item, first_amount)) = colored.first() {
                    if colored.iter().any(|(item, amount)| {
                        item != first_item || (*amount - *first_amount).abs() > f32::EPSILON
                    }) {
                        valid_paint = false;
                        break;
                    }
                    if let Some(existing) = &paint_item {
                        if existing != first_item {
                            valid_paint = false;
                            break;
                        }
                    } else {
                        paint_item = Some(first_item.clone());
                    }
                    paint_amount += *first_amount;
                }
            }
            if !valid_paint {
                continue;
            }
            objects.push(AssemblyObject {
                id,
                name: string_field(entry, "name")
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Assembly Product".to_owned()),
                attached_parts,
                paint: paint_item.map(|item| Ingredient {
                    item,
                    amount: paint_amount,
                }),
                painting_power_kj,
            });
        }
    }
    Ok(objects)
}

fn assembly_starter_power(dir: &Path) -> Result<f32, String> {
    if !dir.is_dir() {
        return Ok(0.0);
    }
    for path in yaml_files(dir)? {
        let Ok(doc) = load_yaml(&path) else { continue };
        for (_, entry) in template_entries(&doc, "BuildableObjectTemplate") {
            if string_field(entry, "type").as_deref() == Some("AL_Start") {
                return Ok(number_field(entry, "alStart_requiredPowerPerAction_kj_str")
                    .unwrap_or(0.0)
                    .max(0.0));
            }
        }
    }
    Ok(0.0)
}

fn merge_ingredients(target: &mut Vec<Ingredient>, additions: &[Ingredient]) {
    for addition in additions {
        if let Some(existing) = target.iter_mut().find(|item| item.item == addition.item) {
            existing.amount += addition.amount;
        } else {
            target.push(addition.clone());
        }
    }
}

fn crafting_machine_profile(
    entry: &Yaml,
    buildable_type: &str,
) -> Option<(MachineRecipeSelector, f32)> {
    let (selector, speed) = match buildable_type {
        "Producer" => {
            let selector = match string_field(entry, "producerRecipeType").as_deref() {
                Some("Tags") => MachineRecipeSelector {
                    tags: string_list(entry, "producer_recipeType_tags"),
                    recipe_ids: Vec::new(),
                },
                Some("Fixed") => MachineRecipeSelector {
                    tags: Vec::new(),
                    recipe_ids: non_empty_string_field(entry, "producer_recipeType_fixed")
                        .into_iter()
                        .collect(),
                },
                _ => return None,
            };
            (
                selector,
                number_field(entry, "producer_recipeTimeModifier_str"),
            )
        }
        "AutoProducer" => (
            MachineRecipeSelector {
                tags: non_empty_string_field(entry, "autoProducer_recipeType_tag")
                    .into_iter()
                    .collect(),
                recipe_ids: Vec::new(),
            },
            number_field(entry, "autoProducer_recipeTimeModifier_str"),
        ),
        "ModularEntityProducer" => (
            MachineRecipeSelector {
                tags: string_list(entry, "modularProducer_recipeType_tags"),
                recipe_ids: non_empty_string_field(
                    entry,
                    "modularProducer_fixedCraftingRecipeIdentifier",
                )
                .into_iter()
                .collect(),
            },
            number_field(entry, "modularProducer_recipeTimeModifier_str"),
        ),
        "BaseStation" => (
            MachineRecipeSelector {
                tags: Vec::new(),
                recipe_ids: string_list(entry, "baseStation_craftingRecipeIdentifier"),
            },
            Some(1.0),
        ),
        _ => return None,
    };

    (!selector.is_empty()).then_some((selector, speed.unwrap_or(1.0).max(0.01)))
}

fn yaml_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let entries =
        fs::read_dir(dir).map_err(|e| format!("Could not read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn load_yaml(path: &Path) -> Result<Yaml, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    YamlLoader::load_from_str(&text)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("{} is empty", path.display()))
}

fn template_entries<'a>(doc: &'a Yaml, root_key: &str) -> Vec<(String, &'a Yaml)> {
    let Some(root) = doc
        .as_hash()
        .and_then(|h| h.get(&Yaml::String(root_key.to_owned())))
        .and_then(Yaml::as_hash)
    else {
        return vec![];
    };
    root.iter()
        .filter_map(|(key, value)| key.as_str().map(|id| (id.to_owned(), value)))
        .collect()
}

fn value<'a>(entry: &'a Yaml, key: &str) -> Option<&'a Yaml> {
    entry.as_hash()?.get(&Yaml::String(key.to_owned()))
}

fn string_field(entry: &Yaml, key: &str) -> Option<String> {
    match value(entry, key)? {
        Yaml::String(value) => Some(value.clone()),
        Yaml::Integer(value) => Some(value.to_string()),
        Yaml::Real(value) => Some(value.clone()),
        _ => None,
    }
}

fn non_empty_string_field(entry: &Yaml, key: &str) -> Option<String> {
    string_field(entry, key).filter(|value| !value.is_empty())
}

fn number_field(entry: &Yaml, key: &str) -> Option<f32> {
    match value(entry, key)? {
        Yaml::Integer(value) => Some(*value as f32),
        Yaml::Real(value) | Yaml::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn bool_field(entry: &Yaml, key: &str) -> Option<bool> {
    match value(entry, key)? {
        Yaml::Boolean(value) => Some(*value),
        Yaml::String(value) if value.eq_ignore_ascii_case("true") => Some(true),
        Yaml::String(value) if value.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn string_list(entry: &Yaml, key: &str) -> Vec<String> {
    value(entry, key)
        .and_then(Yaml::as_vec)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn ingredient_list(entry: &Yaml, key: &str) -> Vec<Ingredient> {
    value(entry, key)
        .and_then(Yaml::as_vec)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Ingredient {
                item: string_field(item, "identifier")?,
                amount: number_field(item, "amount")
                    .or_else(|| number_field(item, "amount_str"))?,
            })
        })
        .filter(|ingredient| ingredient.amount > 0.0)
        .collect()
}

fn ingredient(entry: &Yaml, key: &str) -> Option<Ingredient> {
    let item = value(entry, key)?;
    Some(Ingredient {
        item: string_field(item, "identifier")?,
        amount: number_field(item, "amount").or_else(|| number_field(item, "amount_str"))?,
    })
    .filter(|ingredient| ingredient.amount > 0.0)
}

fn modular_limit(entry: &Yaml, identifier: &str) -> Option<(u32, u32)> {
    value(entry, "modularBuildingLimits")?
        .as_vec()?
        .iter()
        .find(|limit| string_field(limit, "bot_identifier").as_deref() == Some(identifier))
        .map(|limit| {
            (
                number_field(limit, "minAmount").unwrap_or(1.0).max(0.0) as u32,
                number_field(limit, "maxAmount").unwrap_or(1.0).max(0.0) as u32,
            )
        })
}

pub fn humanize_id(id: &str) -> String {
    let words = id
        .trim_start_matches('_')
        .strip_prefix("base_")
        .unwrap_or(id.trim_start_matches('_'));
    words
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn with_yaml(contents: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should follow the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "foundry-planner-machine-test-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test template directory should be created");
            fs::write(path.join("machines.yaml"), contents)
                .expect("test template should be written");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parsed_crafting_profile(yaml: &str) -> Option<(MachineRecipeSelector, f32)> {
        let doc = YamlLoader::load_from_str(yaml)
            .expect("test YAML should parse")
            .into_iter()
            .next()
            .expect("test YAML should contain a document");
        let (_, entry) = template_entries(&doc, "BuildableObjectTemplate")
            .into_iter()
            .next()
            .expect("test YAML should contain a buildable");
        let buildable_type = string_field(entry, "type").unwrap_or_default();
        crafting_machine_profile(entry, &buildable_type)
    }

    #[test]
    fn template_root_override_takes_precedence() {
        let override_root = PathBuf::from("custom/templates");
        let candidates = vec![PathBuf::from("standard/templates")];
        let selected = select_template_root(Some(override_root.clone()), candidates, |_| false)
            .expect("an explicit override should always be selected");
        assert_eq!(selected, override_root);
    }

    #[test]
    fn template_root_uses_first_existing_candidate() {
        let first = PathBuf::from("first/templates");
        let second = PathBuf::from("second/templates");
        let selected =
            select_template_root(None, vec![first, second.clone()], |path| path == second)
                .expect("an existing standard location should be selected");
        assert_eq!(selected, second);
    }

    #[test]
    fn missing_template_root_has_actionable_error() {
        let error = select_template_root(None, vec![PathBuf::from("missing/templates")], |_| false)
            .expect_err("missing candidates should return an error");
        assert!(error.contains(TEMPLATE_ROOT_ENV));
        assert!(error.contains("missing/templates"));
    }

    #[test]
    fn crafting_machine_profiles_use_fields_for_the_active_type() {
        let (selector, speed) = parsed_crafting_profile(
            "BuildableObjectTemplate:\n  auto:\n    type: AutoProducer\n    producer_recipeTimeModifier_str: 9\n    producer_recipeType_tags:\n      - stale\n    autoProducer_recipeTimeModifier_str: 1.5\n    autoProducer_recipeType_tag: advanced_smelter\n",
        )
        .expect("auto producer should load");
        assert_eq!(selector.tags, ["advanced_smelter"]);
        assert!(selector.recipe_ids.is_empty());
        assert!((speed - 1.5).abs() < f32::EPSILON);

        let (selector, speed) = parsed_crafting_profile(
            "BuildableObjectTemplate:\n  modular:\n    type: ModularEntityProducer\n    producer_recipeTimeModifier_str: 9\n    producer_recipeType_tags:\n      - stale\n    modularProducer_fixedCraftingRecipeIdentifier: fixed_recipe\n    modularProducer_recipeTimeModifier_str: 32\n    modularProducer_recipeType_tags:\n      - heavy_caster\n",
        )
        .expect("modular producer should load");
        assert_eq!(selector.tags, ["heavy_caster"]);
        assert_eq!(selector.recipe_ids, ["fixed_recipe"]);
        assert!((speed - 32.0).abs() < f32::EPSILON);

        let (selector, speed) = parsed_crafting_profile(
            "BuildableObjectTemplate:\n  tagged:\n    type: Producer\n    producerRecipeType: Tags\n    producer_recipeTimeModifier_str: 2\n    producer_recipeType_tags:\n      - assembler\n",
        )
        .expect("tagged producer should load");
        assert_eq!(selector.tags, ["assembler"]);
        assert!(selector.recipe_ids.is_empty());
        assert!((speed - 2.0).abs() < f32::EPSILON);

        let (selector, speed) = parsed_crafting_profile(
            "BuildableObjectTemplate:\n  fixed:\n    type: Producer\n    producerRecipeType: Fixed\n    producer_recipeTimeModifier_str: 1\n    producer_recipeType_fixed: fixed_recipe\n",
        )
        .expect("fixed producer should load");
        assert!(selector.tags.is_empty());
        assert_eq!(selector.recipe_ids, ["fixed_recipe"]);
        assert!((speed - 1.0).abs() < f32::EPSILON);

        let (selector, speed) = parsed_crafting_profile(
            "BuildableObjectTemplate:\n  base:\n    type: BaseStation\n    baseStation_craftingRecipeIdentifier:\n      - primitive_plate\n      - primitive_rod\n",
        )
        .expect("base station should load");
        assert!(selector.tags.is_empty());
        assert_eq!(selector.recipe_ids, ["primitive_plate", "primitive_rod"]);
        assert!((speed - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn inactive_and_empty_machine_selectors_are_rejected() {
        assert!(
            parsed_crafting_profile(
                "BuildableObjectTemplate:\n  quarry:\n    type: QuarryBuilding\n    producer_recipeType_tags:\n      - smelter\n    autoProducer_recipeType_tag: advanced_smelter\n",
            )
            .is_none()
        );
        assert!(
            parsed_crafting_profile(
                "BuildableObjectTemplate:\n  empty:\n    type: AutoProducer\n    autoProducer_recipeTimeModifier_str: 2\n    autoProducer_recipeType_tag: \"\"\n",
            )
            .is_none()
        );
    }

    #[test]
    fn direct_processes_require_their_exact_active_type() {
        let templates = TestDirectory::with_yaml(
            "BuildableObjectTemplate:\n  active:\n    type: ResourceConverter\n    nameOverride: Active Converter\n    resourceConverter_powerConsumption_kjPerSec: 100\n    resourceConverter_input_elemental:\n      - identifier: feed\n        amount_str: 2\n    resourceConverter_output_elemental:\n      - identifier: product\n        amount_str: 3\n    resourceConverter_speedBonusModules: []\n    resourceConverter_hasAdjacencyBonus: False\n  stale:\n    type: QuarryBuilding\n    nameOverride: Stale Fields\n    resourceConverter_powerConsumption_kjPerSec: 100\n    resourceConverter_input_elemental:\n      - identifier: feed\n        amount_str: 2\n    resourceConverter_output_elemental:\n      - identifier: product\n        amount_str: 3\n",
        );
        let mut data = GameData::default();
        data.load_machines(&templates.0)
            .expect("test buildables should load");

        assert!(matches!(
            data.machines.first().map(|machine| &machine.kind),
            Some(MachineKind::ResourceConverter(_))
        ));
        assert!(
            data.recipes
                .iter()
                .any(|recipe| recipe.id == "direct:active")
        );
        assert!(!data.machines.iter().any(|machine| machine.id == "stale"));
        assert!(
            !data
                .recipes
                .iter()
                .any(|recipe| recipe.id == "direct:stale")
        );
    }

    #[test]
    fn pipe_intakes_produce_water_at_their_output_transfer_rate() {
        let templates = TestDirectory::with_yaml(
            r#"BuildableObjectTemplate:
  regular:
    type: PipeIntake
    nameOverride: Liquid Intake
    fbm_ioFluidBoxes:
      - isInput: False
        transferRatePerSecond_liter: 500
  pipeline:
    type: PipeIntake
    nameOverride: Liquid Intake (Pipeline)
    fbm_ioFluidBoxes:
      - isInput: False
        transferRatePerSecond_liter: 3000
  input_only:
    type: PipeIntake
    fbm_ioFluidBoxes:
      - isInput: True
        transferRatePerSecond_liter: 500
  missing_rate:
    type: PipeIntake
    fbm_ioFluidBoxes:
      - isInput: False
  zero_rate:
    type: PipeIntake
    fbm_ioFluidBoxes:
      - isInput: False
        transferRatePerSecond_liter: 0
  pump_adapter:
    type: Pump
    fbm_ioFluidBoxes:
      - isInput: False
        transferRatePerSecond_liter: 6000
"#,
        );
        let mut data = GameData::default();
        data.load_machines(&templates.0)
            .expect("test buildables should load");
        data.rebuild_indexes();

        for (machine_id, expected_rate) in [("regular", 30_000.0), ("pipeline", 180_000.0)] {
            let machine = data.machine(machine_id).expect("intake machine");
            assert!(matches!(machine.kind, MachineKind::FixedRate));
            assert_eq!(machine.power_kw, 0.0);

            let recipe = data
                .recipe(&format!("direct:{machine_id}"))
                .expect("intake direct process");
            assert!(recipe.inputs.is_empty());
            assert_eq!(recipe.category, "Extraction");
            assert_eq!(recipe.outputs.len(), 1);
            assert_eq!(recipe.outputs[0].item, "_base_water");
            assert!((recipe.outputs[0].amount - expected_rate).abs() < 0.01);
            assert!(matches!(
                recipe.kind,
                RecipeKind::Direct {
                    anchor: RateAnchor::Output(0),
                    ..
                }
            ));
            let machine_options = data.machine_options(recipe);
            assert_eq!(machine_options.len(), 1);
            assert_eq!(machine_options[0].id, machine_id);
        }

        for ignored in ["input_only", "missing_rate", "zero_rate", "pump_adapter"] {
            assert!(data.machine(ignored).is_none());
            assert!(data.recipe(&format!("direct:{ignored}")).is_none());
        }
        let water_producers = data.recipes_producing("_base_water");
        assert_eq!(water_producers.len(), 2);
        assert!(
            water_producers
                .iter()
                .any(|recipe| recipe.id == "direct:regular")
        );
        assert!(
            water_producers
                .iter()
                .any(|recipe| recipe.id == "direct:pipeline")
        );
    }

    #[test]
    fn machine_recipe_selectors_match_tags_or_explicit_recipe_ids() {
        let recipe = Recipe {
            id: "fixed_recipe".into(),
            name: "Fixed Recipe".into(),
            inputs: Vec::new(),
            outputs: vec![Ingredient {
                item: "output".into(),
                amount: 1.0,
            }],
            time_seconds: 1.0,
            tags: vec!["matching_tag".into()],
            category: String::new(),
            kind: RecipeKind::Crafting,
        };
        let recipe_tags = recipe.tags.iter().map(String::as_str).collect();

        assert!(
            MachineRecipeSelector {
                tags: vec!["matching_tag".into()],
                recipe_ids: Vec::new(),
            }
            .matches(&recipe, &recipe_tags)
        );
        assert!(
            MachineRecipeSelector {
                tags: Vec::new(),
                recipe_ids: vec!["fixed_recipe".into()],
            }
            .matches(&recipe, &recipe_tags)
        );
        assert!(
            !MachineRecipeSelector {
                tags: vec!["other_tag".into()],
                recipe_ids: vec!["other_recipe".into()],
            }
            .matches(&recipe, &recipe_tags)
        );
    }

    #[test]
    fn reads_installed_foundry_data_when_available() {
        let Ok(path) = resolve_template_root() else {
            return;
        };
        let data = GameData::load(&path).expect("configured game data should load");
        let machinery = data.recipe("_base_mic_i").expect("machinery parts recipe");
        assert_eq!(machinery.name, "Machinery Parts");
        assert!(
            machinery
                .inputs
                .iter()
                .any(|i| i.item == "_base_xenoferrite_plates")
        );
        assert!(!data.machine_options(machinery).is_empty());
        assert!(
            !data
                .recipes_producing("_base_xenoferrite_plates")
                .is_empty()
        );
        let tier_three = data
            .recipe("_base_xf_plates_t3")
            .expect("tier 3 plates recipe");
        assert!(
            tier_three
                .inputs
                .iter()
                .any(|i| { i.item == "_base_molten_xf" && (i.amount - 15.0).abs() < f32::EPSILON })
        );

        let furnace = data
            .machine("_base_blast_furnace_base_1")
            .expect("blast furnace building");
        let MachineKind::BlastFurnace(config) = &furnace.kind else {
            panic!("blast furnace should have its specialized configuration");
        };
        assert_eq!((config.min_towers, config.max_towers), (1, 5));
        assert!((config.min_temperature - 1_500.0).abs() < f32::EPSILON);
        assert!((config.optimal_temperature - 2_000.0).abs() < f32::EPSILON);
        assert!((config.output_multiplier - 4.0).abs() < f32::EPSILON);

        let mode = data
            .recipe("_base_bfm_xf")
            .expect("xenoferrite blast furnace mode");
        assert!(matches!(mode.kind, RecipeKind::BlastFurnace { .. }));
        assert!(
            mode.inputs
                .iter()
                .any(|input| input.item == "_base_hot_air")
        );
        assert!(mode.outputs.iter().any(|output| {
            output.item == "_base_waste_gas" && (output.amount - 43_200.0).abs() < 0.01
        }));
        assert!(
            data.recipes_producing("_base_molten_xf")
                .iter()
                .any(|recipe| recipe.id == "_base_bfm_xf")
        );

        let steel_tier_two = data.recipe("_base_steel_t2").expect("tier 2 steel recipe");
        let steel_machines = data.machine_options(steel_tier_two);
        assert_eq!(steel_machines.len(), 1);
        assert_eq!(steel_machines[0].id, "_base_smelter_i");
        assert!((steel_machines[0].speed - 1.5).abs() < f32::EPSILON);

        let firmarlite = data
            .recipe("_base_firmarlite_sheet_t0")
            .expect("firmarlite sheet recipe");
        let lava_smelters = data.machine_options(firmarlite);
        assert_eq!(lava_smelters.len(), 2);
        assert!(lava_smelters.iter().any(|machine| {
            machine.id == "_base_smelter_lava_i" && (machine.speed - 1.0).abs() < f32::EPSILON
        }));
        assert!(lava_smelters.iter().any(|machine| {
            machine.id == "_base_smelter_lava_ii" && (machine.speed - 2.0).abs() < f32::EPSILON
        }));

        let tier_three_machines = data.machine_options(tier_three);
        assert!(
            tier_three_machines
                .iter()
                .any(|machine| machine.id == "_base_casting_building_base")
        );
        let reactor_mix = data
            .recipe("_base_npp_press_internal_01")
            .expect("reactor fuel mix recipe");
        assert!(
            data.machine_options(reactor_mix)
                .iter()
                .any(|machine| machine.id == "_base_npp_pressurizer_base")
        );
        let geothermal = data
            .recipe("_base_geothermal_generator_internal")
            .expect("geothermal steam recipe");
        assert!(
            data.machine_options(geothermal)
                .iter()
                .any(|machine| machine.id == "_base_geothermal_generator_i")
        );
        for recipe_id in [
            "_base_xf_plates_t1_primitive",
            "_base_technum_rods_t1_primitive",
        ] {
            let recipe = data.recipe(recipe_id).expect("base station recipe");
            assert!(
                data.machine_options(recipe)
                    .iter()
                    .any(|machine| machine.id == "_base_baseStation01")
            );
        }
        assert!(data.machine("_base_quarry_building_i").is_none());

        let air = data
            .recipe("direct:_base_air_intake_1")
            .expect("air intake direct process");
        assert!(air
            .outputs
            .iter()
            .any(|output| output.item == "_base_air" && (output.amount - 27_000.0).abs() < 0.01));
        let MachineKind::ResourceConverter(air_config) = &data
            .machine("_base_air_intake_1")
            .expect("air intake building")
            .kind
        else {
            panic!("air intake should be a resource converter");
        };
        assert_eq!((air_config.min_modules, air_config.max_modules), (2, 7));
        assert_eq!(air_config.ignored_modules, 2);

        let boiler = data
            .recipe("direct:_base_boiler_i")
            .expect("boiler direct process");
        assert!(
            boiler
                .inputs
                .iter()
                .any(|input| input.item == "_base_water" && (input.amount - 1_200.0).abs() < 0.01)
        );
        assert!(
            boiler.outputs.iter().any(
                |output| output.item == "_base_steam" && (output.amount - 3_600.0).abs() < 0.01
            )
        );

        for (machine_id, expected_rate) in [
            ("_base_pipe_intake_i", 30_000.0),
            ("_base_pipeline_intake_i", 180_000.0),
        ] {
            let machine = data.machine(machine_id).expect("liquid intake building");
            assert!(matches!(machine.kind, MachineKind::FixedRate));
            assert_eq!(machine.power_kw, 0.0);
            let intake = data
                .recipe(&format!("direct:{machine_id}"))
                .expect("liquid intake direct process");
            assert!(intake.inputs.is_empty());
            assert!(intake.outputs.iter().any(|output| {
                output.item == "_base_water" && (output.amount - expected_rate).abs() < 0.01
            }));
            assert_eq!(data.machine_options(intake)[0].id, machine_id);
        }
        assert!(
            data.recipes_producing("_base_water")
                .iter()
                .any(|recipe| recipe.id == "direct:_base_pipe_intake_i")
        );
        assert!(
            data.recipes_producing("_base_water")
                .iter()
                .any(|recipe| recipe.id == "direct:_base_pipeline_intake_i")
        );

        assert!(
            data.recipe("direct:_base_pumpjack_i:_base_olumite")
                .expect("pumpjack/reservoir joined process")
                .outputs
                .iter()
                .any(|output| output.item == "_base_olumite")
        );
        assert!(
            data.recipe("direct:_base_ore_vein_miner_i:_base_ore_vein_template_xenoferrite")
                .expect("ore-vein miner/terrain joined process")
                .outputs
                .iter()
                .any(|output| output.item == "_base_rubble_xenoferrite")
        );
        let endless_miner = data
            .machine("_base_endless_miner_crystals_i")
            .expect("valid endless miner resource node");
        assert!(endless_miner.required_resource_node.is_some());
        assert!(
            data.recipe("direct:_base_endless_miner_crystals_i")
                .is_some()
        );

        let reactor = data
            .recipe("direct:_base_npp_reactor_base")
            .expect("reactor direct process");
        assert!(reactor.inputs.iter().any(|input| {
            input.item == "_base_npp_reactor_compound_depleted"
                && (input.amount - 180_000.0).abs() < 0.01
        }));
        let turbine = data
            .recipe("direct:_base_npp_steam_turbine_base")
            .expect("steam turbine direct process");
        assert!(turbine.inputs.iter().any(|input| {
            input.item == "_base_npp_steam_high_pressure" && (input.amount - 36_000.0).abs() < 0.01
        }));
        let cooling = data
            .recipe("direct:_base_npp_cooling_tower_base")
            .expect("cooling tower direct process");
        assert!(matches!(
            cooling.kind,
            RecipeKind::Direct {
                anchor: RateAnchor::Input(0),
                ..
            }
        ));

        let robot = data
            .recipe("assembly:_base_admin_robot_i")
            .expect("admin robot assembly-line process");
        assert!((robot.base_rate(&robot.outputs[0]) - 32.0).abs() < 0.01);
        assert!(
            data.machine_options(robot)
                .iter()
                .any(|machine| { matches!(machine.kind, MachineKind::AssemblyLine(_)) })
        );
    }

    #[test]
    fn humanizes_internal_identifiers() {
        assert_eq!(
            humanize_id("_base_xenoferrite_plates"),
            "Xenoferrite Plates"
        );
    }
}

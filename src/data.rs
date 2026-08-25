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

#[derive(Clone, Debug, PartialEq)]
pub enum RecipeKind {
    Crafting,
    BlastFurnace {
        hot_air_input_slot: usize,
        shutdown_slag: Option<String>,
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
pub enum MachineKind {
    Crafting,
    BlastFurnace(BlastFurnaceConfig),
}

#[derive(Clone, Debug)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub speed: f32,
    pub power_kw: f32,
    pub kind: MachineKind,
}

#[derive(Clone, Debug, Default)]
pub struct GameData {
    pub recipes: Vec<Recipe>,
    pub machines: Vec<Machine>,
    pub item_names: HashMap<String, String>,
    pub tag_names: HashMap<String, String>,
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
        data.load_named_templates(&templates.join("ElementTemplate"), "ElementTemplate")?;
        data.load_tags(&templates.join("CraftingTag"))?;
        data.load_machines(&templates.join("BuildableObjectTemplate"))?;
        data.load_recipes(&crafting_dir)?;
        data.load_blast_furnace_modes(&templates.join("BlastFurnaceModeTemplate"))?;

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
                    machine.tags.iter().any(|tag| tags.contains(tag.as_str()))
                }
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
                let tags = string_list(entry, "producer_recipeType_tags");
                let is_blast_furnace =
                    string_field(entry, "type").as_deref() == Some("BlastFurnace");
                if tags.is_empty() && !is_blast_furnace {
                    continue;
                }
                let speed = number_field(entry, "producer_recipeTimeModifier_str")
                    .unwrap_or(1.0)
                    .max(0.01);
                let power_kw = number_field(entry, "energyConsumptionKW_str")
                    .unwrap_or(0.0)
                    .max(0.0);
                let name = string_field(entry, "nameOverride")
                    .filter(|name| !name.is_empty())
                    .or_else(|| self.item_names.get(&id).cloned())
                    .unwrap_or_else(|| humanize_id(&id));
                let kind = if is_blast_furnace {
                    let tower_id = string_field(entry, "blastFurnace_towerModuleBotIdentifier")
                        .unwrap_or_default();
                    let (min_towers, max_towers) =
                        modular_limit(entry, &tower_id).unwrap_or((1, 1));
                    MachineKind::BlastFurnace(BlastFurnaceConfig {
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
                    })
                } else {
                    MachineKind::Crafting
                };
                self.machines.push(Machine {
                    id,
                    name,
                    tags,
                    speed,
                    power_kw,
                    kind,
                });
            }
        }
        Ok(())
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
                MachineKind::Crafting => None,
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

fn number_field(entry: &Yaml, key: &str) -> Option<f32> {
    match value(entry, key)? {
        Yaml::Integer(value) => Some(*value as f32),
        Yaml::Real(value) | Yaml::String(value) => value.parse().ok(),
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
    }

    #[test]
    fn humanizes_internal_identifiers() {
        assert_eq!(
            humanize_id("_base_xenoferrite_plates"),
            "Xenoferrite Plates"
        );
    }
}

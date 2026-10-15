//! Puppets: bindings (the layer tree), handles, behaviors, triggers and hot takes.

use charcraft_ltab::LValue;
use serde::Serialize;

use super::util::{boolean, boolean_or, id_items, int, last_segment, number, numbers, pair, ppath, string};
use crate::classed::{ClassedDoc, class_of};

const MAX_HANDLE_DEPTH: usize = 32;

/// A tag on a layer or handle. A tag listed in `_tTags` is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tag {
    /// The tag id (`Adobe.Face.Front`, `Adobe.LipSynch.Oh`).
    pub name: String,
    /// The flag stored beside the tag. Every tag in the sample files has `false` (the tags Character Animator
    /// assigned itself, from layer names), so `true` is read as "set by the user by hand"; that reading is
    /// unverified.
    pub user_set: bool,
}

/// A keyboard trigger bound to a layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyTrigger {
    /// The key (`None` when unset).
    pub key: Option<String>,
    /// The latch mode number.
    pub latch: i64,
    /// Hide the sibling layers while this one is triggered.
    pub hide_others: bool,
}

/// Where a binding or handle is attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    /// The binding the location belongs to (`None` for the puppet itself).
    pub binding_id: Option<String>,
    /// The location name (`origin`, `__auto_loc`, `__user_origin`).
    pub name: String,
}

/// A decomposed 2D transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Transform {
    /// Translation.
    pub position: [f64; 2],
    /// Scale factors.
    pub scale: [f64; 2],
    /// Rotation angle as stored.
    pub angle: f64,
    /// Shear as stored.
    pub shear: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: [0.0, 0.0], scale: [1.0, 1.0], angle: 0.0, shear: 0.0 }
    }
}

/// What a binding points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BindingKind {
    /// A layer of the artwork (`SkinBinding`).
    Skin,
    /// A child puppet (`PuppetBinding`).
    Puppet,
    /// Any other class.
    Other,
}

/// One child of a puppet: an artwork layer or a child puppet, with its tags, transform and key trigger.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Binding {
    /// Id inside the puppet (`B1`, `B2`, ...).
    pub id: String,
    /// The class name (`SkinBinding`, `PuppetBinding`).
    pub class: Option<String>,
    /// Skin layer, child puppet or other.
    pub kind: BindingKind,
    /// The project path of the target (a layer path inside a skin, or a puppet file).
    pub ppath: Option<String>,
    /// A display name override (`None` means "use the target's name").
    pub name: Option<String>,
    /// Whether the layer is shown by default.
    pub active: bool,
    /// Opacity 0 to 1.
    pub opacity: f64,
    /// Blend mode name.
    pub blend_mode: Option<String>,
    /// Clipped by the layer below.
    pub clipped: bool,
    /// Warps with its parent.
    pub warp_with_parent: bool,
    /// Placement relative to the attach location.
    pub transform: Transform,
    /// Tags on this layer (`Adobe.LipSynch.Oh`, `Adobe.Face.Front`, ...).
    pub tags: Vec<Tag>,
    /// The keyboard trigger.
    pub key_trigger: KeyTrigger,
    /// Where the binding is attached.
    pub attach_to: Option<Location>,
    /// Ids of the triggers that show this layer.
    pub triggers: Vec<String>,
}

/// A handle (rig point) of a puppet and the handles hanging off it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Handle {
    /// Name.
    pub name: String,
    /// Whether the user may edit it.
    pub user_editable: bool,
    /// Its location tag.
    pub location: Option<Location>,
    /// Tags (`Adobe.Dragger3.Draggable`, ...).
    pub tags: Vec<Tag>,
    /// Behavior-written properties, kept as text.
    pub manual_properties: Option<String>,
    /// Handles attached to this one.
    pub children: Vec<Self>,
}

/// A behavior attached to a puppet and the parameter values it overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundBehavior {
    /// Id inside the puppet (`BB1`, ...).
    pub id: String,
    /// The behavior (`Adobe.Blinker.js`).
    pub behavior: String,
    /// Display name as stored (wrapped in snowman characters for stock behaviors).
    pub ui_name: Option<String>,
    /// Shown in the track item properties.
    pub show_in_track_item_properties: bool,
    /// Parameter values that differ from the behavior's defaults, in file order.
    pub params: Vec<(String, LValue)>,
}

/// A trigger inside a trigger group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Trigger {
    /// Id (`Tr1-Lip Sync`).
    pub id: String,
    /// Display name.
    pub name: Option<String>,
    /// Keyboard key.
    pub key: Option<String>,
    /// Latch mode number.
    pub latch: i64,
    /// MIDI channel, if bound.
    pub midi_channel: Option<i64>,
    /// MIDI note, if bound.
    pub midi_note: Option<i64>,
    /// Guid.
    pub guid: Option<String>,
}

/// A group of triggers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TriggerGroup {
    /// Id (`Tg1-Lip Sync`).
    pub id: String,
    /// Display name.
    pub name: Option<String>,
    /// How competing triggers are resolved (`MostRecent`).
    pub priority: Option<String>,
    /// Several triggers of the group may be latched at once.
    pub allow_multiple_latches: bool,
    /// The triggers.
    pub triggers: Vec<Trigger>,
}

/// A hot take: a recorded take replayed when a trigger fires.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HotTake {
    /// Id (`HT1`).
    pub id: String,
    /// Display name.
    pub name: Option<String>,
    /// Shortest take time in seconds.
    pub min_take_time: f64,
    /// Longest take time in seconds.
    pub max_take_time: f64,
    /// Time offset in seconds.
    pub time_offset: f64,
    /// Ids of the triggers that start it.
    pub triggered_by: Vec<String>,
    /// How many recorded streams it holds.
    pub clipboard_items: usize,
    /// The behaviors those streams belong to.
    pub behaviors: Vec<String>,
}

/// A puppet file of the project (`Puppet{...}`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PuppetItem {
    /// The store path (`ProjectItems/Hero`).
    pub path: String,
    /// The file name (`Hero`).
    pub name: String,
    /// `true` for the puppets under `ProjectItems/.Parts`, which are the pieces of a rig.
    pub is_part: bool,
    /// The puppet's guid.
    pub guid: Option<String>,
    /// A display name override.
    pub ui_name: Option<String>,
    /// The top-level puppet of a `.puppet` file.
    pub is_root_in_puppet_file: bool,
    /// The artwork layer this puppet was built from (project path of the skin layer).
    pub artwork: Option<String>,
    /// The layers and child puppets.
    pub bindings: Vec<Binding>,
    /// The handle tree.
    pub origin: Option<Handle>,
    /// Named locations and their coordinates.
    pub tagged_locations: Vec<(String, Vec<f64>)>,
    /// The attached behaviors.
    pub behaviors: Vec<BoundBehavior>,
    /// Trigger groups.
    pub trigger_groups: Vec<TriggerGroup>,
    /// Hot takes.
    pub hot_takes: Vec<HotTake>,
    /// The `_tOptions` table (rigging options).
    pub options: LValue,
}

impl PuppetItem {
    /// The behavior with this id (`Adobe.Blinker.js`).
    #[must_use]
    pub fn behavior(&self, behavior: &str) -> Option<&BoundBehavior> {
        self.behaviors.iter().find(|b| b.behavior == behavior)
    }

    /// The binding with this id.
    #[must_use]
    pub fn binding(&self, id: &str) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.id == id)
    }

    /// Every tag used on a layer of this puppet, with how many layers carry it.
    #[must_use]
    pub fn tag_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for tag in self.bindings.iter().flat_map(|b| &b.tags) {
            if let Some(entry) = counts.iter_mut().find(|(n, _)| *n == tag.name) {
                entry.1 = entry.1.saturating_add(1);
            } else {
                counts.push((tag.name.clone(), 1));
            }
        }
        counts
    }

    /// The name shown for the puppet: its display name, its root handle's name or its file name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.ui_name.as_deref().or_else(|| self.origin.as_ref().map(|h| h.name.as_str()).filter(|n| !n.is_empty() && *n != "origin")).unwrap_or(&self.name)
    }
}

fn tags(value: Option<&LValue>) -> Vec<Tag> {
    value.map(|t| t.named().filter_map(|(name, on)| Some(Tag { name: name.to_owned(), user_set: on.as_bool()? })).collect()).unwrap_or_default()
}

fn location(value: Option<&LValue>) -> Option<Location> {
    let value = value?;
    let name = string(value, "name_")?;
    Some(Location { binding_id: string(value, "bindingId_"), name })
}

fn transform(value: &LValue) -> Transform {
    let Some(d) = value.get("_attachToMatDecomposed").and_then(|m| m.get("_tDecomposed")) else {
        return Transform::default();
    };
    Transform {
        position: pair(d.get("position"), 0.0),
        scale: pair(d.get("scale"), 1.0),
        angle: number(d, "angle"),
        shear: d.get("shear").map(numbers).and_then(|s| s.first().copied()).unwrap_or(0.0),
    }
}

fn binding(id: String, value: &LValue, class: Option<&str>) -> Binding {
    let kind = match class {
        Some("SkinBinding") => BindingKind::Skin,
        Some("PuppetBinding") => BindingKind::Puppet,
        _ => BindingKind::Other,
    };
    let key = value.get("_keyTrigger");
    Binding {
        id,
        class: class.map(str::to_owned),
        kind,
        ppath: ppath(value, "_ppath"),
        name: string(value, "_name"),
        active: boolean_or(value, "_active", true),
        opacity: value.get("_opacity").and_then(LValue::as_f64).unwrap_or(1.0),
        blend_mode: string(value, "_blendMode"),
        clipped: boolean(value, "_bIsClipped"),
        warp_with_parent: boolean(value, "_warpWithParent"),
        transform: transform(value),
        tags: tags(value.get("_tTags")),
        key_trigger: KeyTrigger {
            key: key.and_then(|k| string(k, "_triggerKey")).filter(|k| !k.is_empty()),
            latch: key.map_or(0, |k| int(k, "_triggerLatch")),
            hide_others: key.is_some_and(|k| boolean(k, "_bHideOthersWhenTriggered")),
        },
        attach_to: location(value.get("_tagAttachTo")),
        triggers: value
            .get("_tTriggers")
            .map(|t| t.iter().filter_map(|(k, v)| k.as_str().map(str::to_owned).or_else(|| v.as_str().map(str::to_owned))).collect())
            .unwrap_or_default(),
    }
}

fn handle(value: &LValue, depth: usize) -> Handle {
    let children = if depth < MAX_HANDLE_DEPTH {
        value.get("_aChildren").map(|c| c.iter().map(|(_, h)| handle(h, depth.saturating_add(1))).collect()).unwrap_or_default()
    } else {
        Vec::new()
    };
    Handle {
        name: string(value, "_sName").unwrap_or_default(),
        user_editable: boolean(value, "_bUserEditable"),
        location: location(value.get("_locationTag")),
        tags: tags(value.get("_tTags")),
        manual_properties: string(value, "_manualProperties").filter(|s| !s.is_empty()),
        children,
    }
}

fn behaviors(value: &LValue) -> Vec<BoundBehavior> {
    let Some(array) = value.get("_aBoundBehaviors") else {
        return Vec::new();
    };
    id_items(array)
        .into_iter()
        .map(|(id, b)| {
            let behavior = ppath(b, "_behaviorPPath").map_or_else(String::new, |p| last_segment(&p).to_owned());
            BoundBehavior {
                id,
                behavior,
                ui_name: string(b, "_uiName"),
                show_in_track_item_properties: boolean(b, "_bShowInTrackItemProperties"),
                params: b.get("_sparseParams").map(|p| p.named().map(|(k, v)| (k.to_owned(), v.clone())).collect()).unwrap_or_default(),
            }
        })
        .collect()
}

fn trigger_groups(value: &LValue) -> Vec<TriggerGroup> {
    let Some(groups) = value.get("_oTriggerCollection").and_then(|c| c.get("_idaTriggerGroups")) else {
        return Vec::new();
    };
    id_items(groups)
        .into_iter()
        .map(|(id, g)| TriggerGroup {
            id,
            name: string(g, "_sUiName"),
            priority: string(g, "_priority"),
            allow_multiple_latches: boolean(g, "_bAllowMultipleLatches"),
            triggers: g
                .get("_idaTriggers")
                .map(|t| {
                    id_items(t)
                        .into_iter()
                        .map(|(id, t)| Trigger {
                            id,
                            name: string(t, "_sUiName"),
                            key: string(t, "_triggerKey").filter(|k| !k.is_empty()),
                            latch: int(t, "_triggerLatch"),
                            midi_channel: t.get("_nMidiChannel").and_then(LValue::as_i64),
                            midi_note: t.get("_nMidiNote").and_then(LValue::as_i64),
                            guid: string(t, "_sTriggerGuid"),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

fn hot_takes(value: &LValue) -> Vec<HotTake> {
    let Some(array) = value.get("_idaHotTakes") else {
        return Vec::new();
    };
    id_items(array)
        .into_iter()
        .map(|(id, h)| {
            let items: Vec<&LValue> =
                h.get("_oTakeClipboardItems").and_then(|c| c.get("_aTakeClipboardItems")).map(|a| a.iter().map(|(_, v)| v).collect()).unwrap_or_default();
            HotTake {
                id,
                name: string(h, "_sUIName"),
                min_take_time: number(h, "_minTakeTime"),
                max_take_time: number(h, "_maxTakeTime"),
                time_offset: number(h, "_timeOffset"),
                triggered_by: h
                    .get("_tTriggeredBy")
                    .map(|t| t.named().filter(|(_, on)| on.as_bool() == Some(true)).map(|(k, _)| k.to_owned()).collect())
                    .unwrap_or_default(),
                clipboard_items: items.len(),
                behaviors: items.iter().filter_map(|i| string(i, "_behaviorId")).collect(),
            }
        })
        .collect()
}

/// Decodes a `Puppet{...}` blob stored at `path`.
#[must_use]
pub fn decode(path: &str, doc: &ClassedDoc) -> PuppetItem {
    let value = &doc.value;
    let classes = doc.classes_by_address();
    let bindings =
        value.get("_idaBindings").map(|array| id_items(array).into_iter().map(|(id, b)| binding(id, b, class_of(&classes, b))).collect()).unwrap_or_default();
    PuppetItem {
        path: path.to_owned(),
        name: last_segment(path).to_owned(),
        is_part: path.starts_with("ProjectItems/.Parts/"),
        guid: string(value, "_sGuid"),
        ui_name: string(value, "_sUiName"),
        is_root_in_puppet_file: boolean(value, "_isRootInPuppetFile"),
        artwork: ppath(value, "_originatingSkinPPath0"),
        bindings,
        origin: value.get("_origin").map(|h| handle(h, 0)),
        tagged_locations: value.get("_tTaggedLocations").map(|t| t.named().map(|(k, v)| (k.to_owned(), numbers(v))).collect()).unwrap_or_default(),
        behaviors: behaviors(value),
        trigger_groups: trigger_groups(value),
        hot_takes: hot_takes(value),
        options: value.get("_tOptions").cloned().unwrap_or(LValue::Nil),
    }
}

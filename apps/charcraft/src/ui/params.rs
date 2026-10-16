//! Character Animator style display of behavior parameters.
//!
//! Friendly names, units, sliders with ranges, and the advanced ones tucked away. The raw key stays what the MCP
//! commands and the files use; only the display changes.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use super::theme::TEXT_WEAK;
use super::widgets::scrub_style;
use crate::view::Action;

/// How a parameter is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    /// The name in the panel.
    pub label: &'static str,
    /// The unit written after the number.
    pub unit: &'static str,
    /// Slider range when it has one (values outside can still be typed).
    pub range: Option<(f64, f64)>,
    /// Advanced and internal settings are listed under "Advanced".
    pub advanced: bool,
}

const fn plain(label: &'static str) -> Spec {
    Spec { label, unit: "", range: None, advanced: false }
}

const fn percent(label: &'static str) -> Spec {
    Spec { label, unit: "%", range: Some((0.0, 200.0)), advanced: false }
}

const fn degrees(label: &'static str, lo: f64, hi: f64) -> Spec {
    Spec { label, unit: "\u{b0}", range: Some((lo, hi)), advanced: false }
}

const fn amount(label: &'static str, lo: f64, hi: f64) -> Spec {
    Spec { label, unit: "", range: Some((lo, hi)), advanced: false }
}

const fn timed(label: &'static str, unit: &'static str, hi: f64) -> Spec {
    Spec { label, unit, range: Some((0.0, hi)), advanced: false }
}

const fn advanced(label: &'static str) -> Spec {
    Spec { label, unit: "", range: None, advanced: true }
}

/// `camelCase` or `snake_case` as capitalised words, with a trailing `Factor` read as strength.
fn humanize(key: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in key.replace('_', " ").chars() {
        if c.is_uppercase() && previous_lower {
            out.push(' ');
        }
        previous_lower = c.is_lowercase() || c.is_ascii_digit();
        out.push(c);
    }
    let words: Vec<String> = out
        .split_whitespace()
        .map(|w| {
            let w = if w == "Factor" { "Strength" } else { w };
            let mut chars = w.chars();
            chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
        })
        .collect();
    words.join(" ")
}

/// The display of a parameter by its Character Animator key; `None` for keys without a hand-made entry.
fn known(key: &str) -> Option<Spec> {
    Some(match key {
        // Face.
        "blinkOnly" => plain("Blink Only"),
        "downEyebrowsTilt" => degrees("Down Eyebrows Tilt", -90.0, 90.0),
        "upEyebrowsTilt" => degrees("Up Eyebrows Tilt", -90.0, 90.0),
        "eyeFactor" => percent("Eye Strength"),
        "eyebrowFactor" => percent("Eyebrow Strength"),
        "headPosFactor" => percent("Head Position Strength"),
        "headPosVerticalFactor" => percent("Head Vertical Position Strength"),
        "headRotFactor" => percent("Head Rotation Strength"),
        "headScaleFactor" => percent("Head Scale Strength"),
        "mouthFactor" => percent("Mouth Strength"),
        "parallaxFactor" => percent("Parallax Strength"),
        "moveEyebrowsTogether" => plain("Move Eyebrows Together"),
        "poseFilteringLevel" => amount("Pose Filtering", 0.0, 100.0),
        "smoothingRate" => amount("Smoothing", 0.0, 100.0),
        "enablePoseToPoseAdvancedParams" => advanced("Pose-to-Pose Settings"),
        "eyebrowPosMeanThresh" => advanced("Eyebrow Position Mean Threshold"),
        "eyebrowPosVarThresh" => advanced("Eyebrow Position Variance Threshold"),
        "headPosMeanThresh" => advanced("Head Position Mean Threshold"),
        "headPosVarThresh" => advanced("Head Position Variance Threshold"),
        "headRotMeanThresh" => advanced("Head Rotation Mean Threshold"),
        "headRotVarThresh" => advanced("Head Rotation Variance Threshold"),
        "filterNumSamples" => advanced("Filter Samples"),
        "maxInterval" => advanced("Longest Pose Interval"),
        "minInterval" => advanced("Shortest Pose Interval"),
        "minPoseDuration" => advanced("Shortest Pose Duration"),
        // Lip Sync.
        "jawFactor" => percent("Jaw Movement"),
        "useTransitionMouthShapes" => plain("Use Transition Mouth Shapes"),
        "faceTrackerSetMouthSilenceDurationThreshInMilliseconds" => timed("Mouth Silence Threshold", "ms", 2000.0),
        // Eye Gaze.
        "eyeGazeFactor" => percent("Eye Gaze Strength"),
        "keyboardEyeGazeFactor" => percent("Keyboard Gaze Strength"),
        "mouseEyeGazeFactor" => percent("Mouse Gaze Strength"),
        "leftRightScaleFactor" => percent("Left and Right Scale"),
        "upDownScaleFactor" => percent("Up and Down Scale"),
        "offAxisScaleFactor" => percent("Off-Axis Scale"),
        "minGazeDuration" => timed("Minimum Gaze Duration", "s", 10.0),
        "minSnapOffsetDist" => amount("Minimum Snap Distance", 0.0, 100.0),
        "numEyeDartDirections" => amount("Eye Dart Directions", 0.0, 16.0),
        "rectangularEyes" => plain("Rectangular Eyes"),
        "snapEyeGaze" => plain("Snap Eye Gaze"),
        "scaleTransformsV2" => advanced("Scale Transforms"),
        // Transform.
        "positionX" => plain("Position X"),
        "positionY" => plain("Position Y"),
        "scaleX" => percent("Scale X"),
        "scaleY" => percent("Scale Y"),
        "uniformScale" => plain("Uniform Scale"),
        "skewAxis" => degrees("Skew Axis", -180.0, 180.0),
        "groupOpacity" => Spec { label: "Opacity", unit: "%", range: Some((0.0, 100.0)), advanced: false },
        "anchorPtX" => plain("Anchor Point X"),
        "anchorPtY" => plain("Anchor Point Y"),
        "applyPosition" => plain("Apply Position"),
        "applyRotation" => plain("Apply Rotation"),
        "applyScale" => plain("Apply Scale"),
        "applySkew" => plain("Apply Skew"),
        // Dragger and physics.
        "handleReleaseDuration" => timed("Release Duration", "ms", 5000.0),
        "onMouseUp" => plain("When the Mouse Is Released"),
        "showTargets" => plain("Show Targets"),
        "useBehaviorRelativeHandlePathsV2" => advanced("Use Relative Handle Paths"),
        "gravityStrength" => amount("Gravity Strength", 0.0, 500.0),
        "gravityDirection" => degrees("Gravity Direction", -180.0, 180.0),
        "windStrength" => amount("Wind Strength", 0.0, 500.0),
        "windDirection" => degrees("Wind Direction", -180.0, 180.0),
        "windVariation" => amount("Wind Variation", 0.0, 100.0),
        "motorStrength" => amount("Motor Strength", 0.0, 500.0),
        "wallCollide" => plain("Collide with Walls"),
        "startWhen" => plain("Start When"),
        "GeometryType" => plain("Geometry Type"),
        // Walk and Breathe.
        "leftKey" => plain("Left Key"),
        "rightKey" => plain("Right Key"),
        "strideLength" => amount("Stride Length", 0.0, 500.0),
        "armSwing" => percent("Arm Swing"),
        "legSwing" => percent("Leg Swing"),
        "moveBody" => plain("Move Body"),
        "easeRate" => amount("Ease Rate", 0.0, 100.0),
        "inhaleRatio" => Spec { label: "Inhale Ratio", unit: "%", range: Some((0.0, 100.0)), advanced: false },
        // Triggers.
        "audioVolume" => Spec { label: "Audio Volume", unit: "%", range: Some((0.0, 100.0)), advanced: false },
        "maxReplayDepth" => amount("Maximum Replay Depth", 0.0, 20.0),
        "onlyPlayAudioWhenStopped" => plain("Only Play Audio When Stopped"),
        // Arm IK.
        "globalStrength" => percent("Global Strength"),
        "elbowStrength" => percent("Elbow Strength"),
        "shoulderAngleLimit" => degrees("Shoulder Angle Limit", 0.0, 180.0),
        "ankleFlexibility" => percent("Ankle Flexibility"),
        "toeStickiness" => percent("Toe Stickiness"),
        "autoBend" => plain("Auto Bend"),
        "autoGround" => plain("Auto Ground"),
        "initFeet2" | "applyTo2" => advanced("Setup"),
        _ => return None,
    })
}

/// The label, unit, range and advanced flag of a parameter of a behavior (the behavior id is for the future: the
/// same key means the same thing in every behavior today).
#[must_use]
pub fn spec(_behavior: &str, key: &str) -> (String, Spec) {
    if let Some(spec) = known(key) {
        return (spec.label.to_owned(), spec);
    }
    let label = humanize(key);
    let hidden = key.starts_with("Trim_") || key.ends_with("Thresh") || key.ends_with("V2");
    (label, Spec { label: "", unit: "", range: None, advanced: hidden })
}

/// Whether a parameter belongs under "Advanced".
#[must_use]
pub fn is_advanced(behavior: &str, key: &str) -> bool {
    spec(behavior, key).1.advanced
}

/// One parameter row: the friendly label and the control, sending `track.behavior` commands with the raw key.
pub fn row(ui: &mut Ui, track: usize, info: &charcraft_behaviors::api::BehaviorInfo, key: &str, value: &Value, actions: &mut Vec<Action>) {
    let (index, behavior) = (info.index, info.id.as_str());
    let (label, spec) = spec(behavior, key);
    let set = |value: Value| {
        Action::Run("track.behavior", json!({ "track": track, "op": charcraft_behaviors::api::StackOp::SetParam { index, name: key.to_owned(), value } }))
    };
    ui.horizontal(|ui| {
        ui.add_sized([128.0, 16.0], egui::Label::new(RichText::new(label).color(TEXT_WEAK)).truncate()).on_hover_text(key);
        match value {
            Value::Bool(b) => {
                let mut on = *b;
                if ui.checkbox(&mut on, "").changed() {
                    actions.push(set(json!(on)));
                }
            }
            Value::Number(n) => {
                let current = n.as_f64().unwrap_or(0.0);
                let id = ui.id().with(("param", track, index, key));
                let mut edited: f64 = ui.data(|d| d.get_temp(id)).unwrap_or(current);
                let speed = (current.abs() * 0.01).max(0.01);
                let response = scrub_style(ui, |ui| match spec.range {
                    Some((lo, hi)) => {
                        ui.spacing_mut().slider_width = 88.0;
                        let mut slider = egui::Slider::new(&mut edited, lo..=hi).clamping(egui::SliderClamping::Never).trailing_fill(true);
                        if !spec.unit.is_empty() {
                            slider = slider.suffix(spec.unit);
                        }
                        ui.add(slider)
                    }
                    None => ui.add(egui::DragValue::new(&mut edited).speed(speed).suffix(spec.unit)),
                });
                if response.changed() {
                    ui.data_mut(|d| d.insert_temp(id, edited));
                }
                if (response.drag_stopped() || response.lost_focus()) && ui.data(|d| d.get_temp::<f64>(id)).is_some() {
                    ui.data_mut(|d| d.remove_temp::<f64>(id));
                    let as_int = n.is_i64() || n.is_u64();
                    actions.push(set(if as_int { json!(anim_core::num::round_f64_to_i64(edited)) } else { json!(edited) }));
                }
            }
            other => {
                let choices = info.docs.iter().find(|d| d.name == key).map_or(&[][..], |d| d.choices);
                super::rig::param_editor(ui, track, index, key, other, choices, actions);
            }
        }
    });
}

/// One behavior as a collapsible group, as in Character Animator.
///
/// A disclosure triangle, the red arm dot, the name (drag it onto another behavior to reorder), and on the right the
/// on/off eye. Right-click for the menu with Remove.
pub fn behavior_block(
    ui: &mut Ui,
    track: usize,
    info: &charcraft_behaviors::api::BehaviorInfo,
    count: usize,
    armed: &std::collections::BTreeSet<charcraft_record::ParamGroup>,
    open: bool,
    actions: &mut Vec<Action>,
) {
    use anim_ui::icons::Icon;
    use charcraft_behaviors::api::StackOp;

    let groups = super::record::arm_groups(&info.id);
    let id = ui.make_persistent_id(("behavior_block", track, info.index));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let header = state.show_header(ui, |ui| {
        let on = !groups.is_empty() && groups.iter().all(|g| armed.contains(g));
        if groups.is_empty() {
            ui.add_space(16.0);
        } else if super::widgets::arm_dot(ui, on, "Arm: record this behavior's input into the next take").clicked() {
            actions.push(Action::Run("record.arm", json!({ "groups": groups, "armed": !on })));
        }
        let name = RichText::new(&info.name).family(super::theme::semibold());
        let name = if info.enabled { name } else { name.color(TEXT_WEAK) };
        ui.dnd_drag_source(id.with("drag"), info.index, |ui| {
            ui.label(name);
        })
        .response
        .on_hover_text("Drag onto another behavior to reorder");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let eye = if info.enabled { Icon::Eye } else { Icon::EyeOff };
            let tip = if info.enabled { "Behavior on: click to turn off" } else { "Behavior off: click to turn on" };
            if anim_ui::widgets::icon_button(ui, egui::vec2(18.0, 16.0), eye, false, true, tip).clicked() {
                actions.push(Action::Run("track.behavior", json!({ "track": track, "op": StackOp::SetEnabled { index: info.index, enabled: !info.enabled } })));
            }
        });
    });
    let (_, header_inner, _) = header.body(|ui| {
        if let Some(map) = info.params.as_object() {
            for (name, value) in map.iter().filter(|(k, _)| !is_advanced(&info.id, k)) {
                row(ui, track, info, name, value, actions);
            }
            if map.keys().any(|k| is_advanced(&info.id, k)) {
                egui::CollapsingHeader::new(RichText::new("Advanced").color(TEXT_WEAK)).id_salt(("advanced", track, info.index)).default_open(false).show(
                    ui,
                    |ui| {
                        for (name, value) in map.iter().filter(|(k, _)| is_advanced(&info.id, k)) {
                            row(ui, track, info, name, value, actions);
                        }
                    },
                );
            }
        }
    });
    let response = header_inner.response;
    if let Some(from) = response.dnd_release_payload::<usize>()
        && *from != info.index
    {
        actions.push(Action::Run("track.behavior", json!({ "track": track, "op": StackOp::Move { from: *from, to: info.index } })));
    }
    response.context_menu(|ui| {
        if ui.add_enabled(info.index > 0, egui::Button::new("Move Up")).clicked() {
            actions.push(Action::Run("track.behavior", json!({ "track": track, "op": StackOp::Move { from: info.index, to: info.index.saturating_sub(1) } })));
            ui.close();
        }
        if ui.add_enabled(info.index.saturating_add(1) < count, egui::Button::new("Move Down")).clicked() {
            actions.push(Action::Run("track.behavior", json!({ "track": track, "op": StackOp::Move { from: info.index, to: info.index.saturating_add(1) } })));
            ui.close();
        }
        if ui.button("Reset Parameters").clicked() {
            if let Some(entry) = charcraft_behaviors::api::catalog().into_iter().find(|c| c.id == info.id) {
                actions.push(Action::Run("track.behavior", json!({ "track": track, "op": StackOp::SetParams { index: info.index, params: entry.params } })));
            }
            ui.close();
        }
        if ui.button("Remove Behavior").clicked() {
            actions.push(Action::Run("track.behavior", json!({ "track": track, "op": StackOp::Remove { index: info.index } })));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_get_friendly_names_and_units() {
        let (label, spec) = spec("Adobe.FaceTracker2.js", "headPosFactor");
        assert_eq!(label, "Head Position Strength");
        assert_eq!(spec.unit, "%");
        assert!(spec.range.is_some());
        assert_eq!(super::spec("x", "downEyebrowsTilt").1.unit, "\u{b0}");
    }

    #[test]
    fn unknown_keys_are_made_readable_and_internal_ones_are_advanced() {
        assert_eq!(spec("x", "someNewFactor").0, "Some New Strength");
        assert_eq!(spec("x", "Trim_Ah").0, "Trim Ah");
        assert!(is_advanced("x", "Trim_Ah"));
        assert!(is_advanced("x", "headPosMeanThresh"));
        assert!(!is_advanced("x", "blinkOnly"));
    }

    #[test]
    fn every_catalog_parameter_has_a_label() {
        for entry in charcraft_behaviors::api::catalog() {
            let Some(map) = entry.params.as_object() else { continue };
            for key in map.keys() {
                assert!(!spec(&entry.id, key).0.is_empty(), "{} {key}", entry.id);
            }
        }
    }
}

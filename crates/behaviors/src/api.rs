//! The tool surface: queries and edits of a behavior stack as plain serde values.
//!
//! This crate has no server of its own. The application maps these to MCP tools one to one (the argument and
//! result types are all `Serialize` and `Deserialize`, so each tool's JSON schema is the type's shape):
//!
//! | tool | argument | result |
//! |---|---|---|
//! | `behaviors.catalog` | none | [`catalog`]: every behavior this program implements, with default parameters and docs |
//! | `behaviors.stack` | none | [`StackInfo`] from [`stack_info`] |
//! | `behaviors.edit` | [`StackOp`] | [`StackInfo`] after [`apply`] |
//! | `behaviors.simulate` | [`Script`](crate::Script) and an optional frame step | `Vec<SimFrame>` from [`simulate`](crate::simulate) |
//! | `behaviors.pose` | [`Inputs`](crate::Inputs) | the [`PuppetPose`](charcraft_puppet::PuppetPose) one tick gives ([`BehaviorStack::step`]) |
//!
//! `behaviors.simulate` and `behaviors.pose` return poses; the puppet crate's `puppet.render` tool draws them.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::behavior::{ParamDoc, set_param};
use crate::behaviors;
use crate::error::{Error, Result};
use crate::input::Inputs;
use crate::stack::BehaviorStack;

/// One edit of a stack. Plain serde, tagged by `op`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StackOp {
    /// Adds a behavior by id, with optional parameters, at a position (the end when absent).
    Add {
        /// The behavior id, from the catalog.
        behavior: String,
        /// Parameters to overlay on the defaults.
        #[serde(default)]
        params: Option<Value>,
        /// Position.
        #[serde(default)]
        at: Option<usize>,
    },
    /// Removes the behavior at a position.
    Remove {
        /// Position.
        index: usize,
    },
    /// Moves a behavior.
    Move {
        /// Where it is.
        from: usize,
        /// Where it goes.
        to: usize,
    },
    /// Sets one parameter.
    SetParam {
        /// Position.
        index: usize,
        /// Parameter name, as in Character Animator files.
        name: String,
        /// The value.
        value: Value,
    },
    /// Overlays several parameters.
    SetParams {
        /// Position.
        index: usize,
        /// A JSON object of parameters.
        params: Value,
    },
    /// Puts every parameter of a behavior back to its default (the behavior stays on or off as it was).
    ResetParams {
        /// Position.
        index: usize,
    },
    /// Switches a behavior on or off.
    SetEnabled {
        /// Position.
        index: usize,
        /// The new state.
        enabled: bool,
    },
    /// Changes the seed of the random streams (restarts the stack).
    SetSeed {
        /// The seed.
        seed: u64,
    },
    /// Restarts every behavior.
    Reset,
}

/// What a tool shows about one behavior in a stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BehaviorInfo {
    /// Position in the stack.
    pub index: usize,
    /// Character Animator's id.
    pub id: String,
    /// The name in the behaviors panel.
    pub name: String,
    /// Whether it runs.
    pub enabled: bool,
    /// Parameters.
    pub params: Value,
    /// What the parameters mean.
    pub docs: Vec<ParamDoc>,
}

/// What a tool shows about a stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackInfo {
    /// The seed of the random streams.
    pub seed: u64,
    /// The behaviors in order.
    pub behaviors: Vec<BehaviorInfo>,
    /// Handles the rig offers: `(key, kind)`.
    pub handles: Vec<(String, String)>,
}

/// One kind of behavior that can be added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogEntry {
    /// Character Animator's id.
    pub id: String,
    /// The name in the behaviors panel.
    pub name: String,
    /// Default parameters.
    pub params: Value,
    /// What the parameters mean.
    pub docs: Vec<ParamDoc>,
}

/// Every behavior this program implements.
#[must_use]
pub fn catalog() -> Vec<CatalogEntry> {
    behaviors::IDS
        .iter()
        .filter_map(|id| behaviors::create(id).ok())
        .map(|b| CatalogEntry { id: b.id().to_owned(), name: b.name().to_owned(), params: b.params(), docs: b.docs().to_vec() })
        .collect()
}

/// Describes a stack.
#[must_use]
pub fn stack_info(stack: &BehaviorStack) -> StackInfo {
    StackInfo {
        seed: stack.seed(),
        behaviors: stack
            .slots()
            .iter()
            .enumerate()
            .map(|(index, s)| BehaviorInfo {
                index,
                id: s.behavior.id().to_owned(),
                name: s.behavior.name().to_owned(),
                enabled: s.enabled,
                params: s.behavior.params(),
                docs: s.behavior.docs().to_vec(),
            })
            .collect(),
        handles: stack.rig().handles.iter().map(|h| (h.key.clone(), format!("{:?}", h.kind))).collect(),
    }
}

/// Applies an edit and returns the stack's new description.
///
/// # Errors
///
/// [`Error::UnknownBehavior`], [`Error::BadParams`], [`Error::NoSuchParam`] or [`Error::NoSuchSlot`]. A failed edit
/// leaves the stack as it was.
pub fn apply(stack: &mut BehaviorStack, op: &StackOp) -> Result<StackInfo> {
    match op {
        StackOp::Add { behavior, params, at } => {
            let b = match params {
                Some(p) => behaviors::create_with(behavior, p)?,
                None => behaviors::create(behavior)?,
            };
            stack.insert(at.unwrap_or(usize::MAX), b);
        }
        StackOp::Remove { index } => {
            stack.remove(*index)?;
        }
        StackOp::Move { from, to } => stack.move_slot(*from, *to)?,
        StackOp::SetParam { index, name, value } => {
            let slot = stack.slot_mut(*index).ok_or(Error::NoSuchSlot(*index))?;
            set_param(slot.behavior.as_mut(), name, value.clone())?;
        }
        StackOp::SetParams { index, params } => {
            let slot = stack.slot_mut(*index).ok_or(Error::NoSuchSlot(*index))?;
            slot.behavior.set_params(params)?;
        }
        StackOp::ResetParams { index } => {
            let slot = stack.slot_mut(*index).ok_or(Error::NoSuchSlot(*index))?;
            let defaults = behaviors::create(slot.behavior.id())?.params();
            slot.behavior.set_params(&defaults)?;
        }
        StackOp::SetEnabled { index, enabled } => {
            stack.slot_mut(*index).ok_or(Error::NoSuchSlot(*index))?.enabled = *enabled;
        }
        StackOp::SetSeed { seed } => stack.set_seed(*seed),
        StackOp::Reset => stack.reset(),
    }
    Ok(stack_info(stack))
}

/// The pose one tick gives (the `behaviors.pose` tool).
pub fn pose_at(stack: &mut BehaviorStack, inputs: &Inputs) -> PuppetPose {
    stack.step(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rig::Rig;

    #[test]
    fn the_catalog_lists_every_behavior_with_documented_parameters() {
        let c = catalog();
        assert_eq!(c.len(), behaviors::IDS.len());
        let blinker = c.iter().find(|e| e.id == "Adobe.Blinker.js").unwrap();
        assert_eq!(blinker.params["rate"], 30.0);
        assert_eq!(blinker.name, "Auto Blink");
        assert!(blinker.docs.iter().any(|d| d.name == "rate"));
    }

    #[test]
    fn edits_apply_and_failures_change_nothing() {
        let mut s = BehaviorStack::new(Rig::default());
        let info = apply(&mut s, &StackOp::Add { behavior: "Adobe.Blinker.js".into(), params: Some(serde_json::json!({"rate": 60})), at: None }).unwrap();
        assert_eq!(info.behaviors.len(), 1);
        assert_eq!(info.behaviors[0].params["rate"], 60.0);
        apply(&mut s, &StackOp::SetParam { index: 0, name: "length".into(), value: serde_json::json!(120) }).unwrap();

        let before = stack_info(&s);
        assert!(matches!(apply(&mut s, &StackOp::SetParam { index: 0, name: "nope".into(), value: Value::Null }), Err(Error::NoSuchParam { .. })));
        assert!(matches!(apply(&mut s, &StackOp::SetParam { index: 0, name: "rate".into(), value: serde_json::json!("fast") }), Err(Error::BadParams { .. })));
        assert!(matches!(apply(&mut s, &StackOp::Remove { index: 4 }), Err(Error::NoSuchSlot(4))));
        assert!(matches!(apply(&mut s, &StackOp::Add { behavior: "x".into(), params: None, at: None }), Err(Error::UnknownBehavior(_))));
        assert_eq!(stack_info(&s), before);
        assert_eq!(before.behaviors[0].params["length"], 120.0);
    }

    #[test]
    fn reset_params_restores_the_defaults_and_keeps_the_switch() {
        let mut s = BehaviorStack::new(Rig::default());
        apply(&mut s, &StackOp::Add { behavior: "Adobe.Blinker.js".into(), params: Some(serde_json::json!({"rate": 60})), at: None }).unwrap();
        apply(&mut s, &StackOp::SetEnabled { index: 0, enabled: false }).unwrap();
        let info = apply(&mut s, &StackOp::ResetParams { index: 0 }).unwrap();
        assert_eq!(info.behaviors[0].params["rate"], 30.0);
        assert!(!info.behaviors[0].enabled);
        assert!(matches!(apply(&mut s, &StackOp::ResetParams { index: 3 }), Err(Error::NoSuchSlot(3))));
    }

    #[test]
    fn ops_survive_json() {
        let ops = vec![
            StackOp::ResetParams { index: 1 },
            StackOp::Add { behavior: "Adobe.Dragger3.js".into(), params: None, at: Some(0) },
            StackOp::Move { from: 0, to: 2 },
            StackOp::SetEnabled { index: 1, enabled: false },
            StackOp::SetSeed { seed: 9 },
            StackOp::Reset,
        ];
        let json = serde_json::to_string(&ops).unwrap();
        assert!(json.contains(r#""op":"set_enabled""#));
        assert_eq!(serde_json::from_str::<Vec<StackOp>>(&json).unwrap(), ops);
    }
}

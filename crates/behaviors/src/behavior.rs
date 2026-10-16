//! The behavior contract.

use std::fmt::Debug;

use charcraft_puppet::PuppetPose;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::input::Inputs;
use crate::rig::Rig;

/// Everything a behavior sees while it steps.
#[derive(Debug, Clone, Copy)]
pub struct Cx<'a> {
    /// This tick's inputs.
    pub inputs: &'a Inputs,
    /// The puppet's description.
    pub rig: &'a Rig,
    /// Seconds since the stack's first tick (or since the last seek).
    pub time: f64,
    /// Seconds since the previous tick, 0 on the first.
    pub dt: f64,
}

/// Documentation of one parameter, for editors and tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ParamDoc {
    /// The name as Character Animator files spell it.
    pub name: &'static str,
    /// The unit (`ms`, `%`, `deg`, ...), empty for plain numbers and switches.
    pub unit: &'static str,
    /// What it does here.
    pub doc: &'static str,
    /// For a parameter that holds one of a few words: the words it accepts. Empty for everything else.
    pub choices: &'static [&'static str],
}

/// A behavior: a deterministic function of its parameters, its own state and the inputs, adding to a pose.
///
/// Behaviors run in stack order. Each reads the pose the earlier ones built and adds its contribution, so an
/// earlier `Face` head turn is seen by a later `Physics` that swings hair from it.
pub trait Behavior: Debug + Send {
    /// The id used in Character Animator files (`Adobe.Blinker.js`).
    fn id(&self) -> &'static str;
    /// The name shown in the behaviors panel.
    fn name(&self) -> &'static str;
    /// The parameters as a JSON object, named exactly as in Character Animator files.
    fn params(&self) -> Value;
    /// Replaces parameters. Missing names keep the behavior's defaults; unknown names are ignored.
    ///
    /// # Errors
    ///
    /// [`Error::BadParams`] when a value has the wrong type.
    fn set_params(&mut self, params: &Value) -> Result<()>;
    /// What the parameters mean.
    fn docs(&self) -> &'static [ParamDoc];
    /// Adds this tick's contribution to the pose.
    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose);
    /// Forgets all state and restarts the random stream from `seed`.
    fn reset(&mut self, seed: u64);
    /// A copy, state included.
    fn clone_box(&self) -> Box<dyn Behavior>;
}

impl Clone for Box<dyn Behavior> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Sets one parameter of a behavior by name, checking that the name exists and the value has the right type.
///
/// # Errors
///
/// [`Error::NoSuchParam`] or [`Error::BadParams`].
pub fn set_param(behavior: &mut dyn Behavior, name: &str, value: Value) -> Result<()> {
    let Value::Object(mut map) = behavior.params() else {
        return Err(Error::BadParams { behavior: behavior.id().to_owned(), message: "parameters are not an object".to_owned() });
    };
    if !map.contains_key(name) {
        return Err(Error::NoSuchParam { behavior: behavior.id().to_owned(), param: name.to_owned() });
    }
    map.insert(name.to_owned(), value);
    behavior.set_params(&Value::Object(map))
}

/// Overlays `overrides` (an object) onto `base` (an object) and returns the result.
#[must_use]
pub fn overlay(base: &Value, overrides: &Value) -> Value {
    let mut out: Map<String, Value> = base.as_object().cloned().unwrap_or_default();
    if let Some(o) = overrides.as_object() {
        for (k, v) in o {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// Reads a parameter object into a typed struct, naming the behavior in the error.
///
/// # Errors
///
/// [`Error::BadParams`].
pub fn read_params<P: serde::de::DeserializeOwned>(behavior: &str, value: &Value) -> Result<P> {
    serde_json::from_value(value.clone()).map_err(|e| Error::BadParams { behavior: behavior.to_owned(), message: e.to_string() })
}

/// Implements the parts of [`Behavior`] that only depend on a `params` field, inside an `impl Behavior` block.
macro_rules! behavior_common {
    ($id:expr, $name:expr, $docs:expr) => {
        fn id(&self) -> &'static str {
            $id
        }
        fn name(&self) -> &'static str {
            $name
        }
        fn params(&self) -> serde_json::Value {
            serde_json::to_value(&self.params).unwrap_or(serde_json::Value::Null)
        }
        fn set_params(&mut self, params: &serde_json::Value) -> $crate::error::Result<()> {
            let merged = $crate::behavior::overlay(&self.params(), params);
            self.params = $crate::behavior::read_params($id, &merged)?;
            Ok(())
        }
        fn docs(&self) -> &'static [$crate::behavior::ParamDoc] {
            $docs
        }
        fn clone_box(&self) -> Box<dyn Behavior> {
            Box::new(self.clone())
        }
    };
}
pub(crate) use behavior_common;

/// Shorthand for building a [`ParamDoc`] table.
macro_rules! docs {
    ($($name:literal, $unit:literal, $doc:literal $(, [$($choice:literal),* $(,)?])?;)*) => {
        &[$($crate::behavior::ParamDoc { name: $name, unit: $unit, doc: $doc, choices: &[$($($choice),*)?] }),*]
    };
}
pub(crate) use docs;

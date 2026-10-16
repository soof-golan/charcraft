//! The behavior stack: behaviors in order, turning inputs over time into poses.

use anim_core::Tick;
use charcraft_project::{BehaviorDefaults, BoundBehavior};
use charcraft_puppet::PuppetPose;

use crate::behavior::{Behavior, Cx};
use crate::behaviors;
use crate::error::{Error, Result};
use crate::input::Inputs;
use crate::rig::Rig;
use crate::util::prune_identity;

/// One behavior in the stack.
#[derive(Debug, Clone)]
pub struct Slot {
    /// A disabled behavior stays in the stack but adds nothing.
    pub enabled: bool,
    /// The behavior.
    pub behavior: Box<dyn Behavior>,
}

/// An ordered list of behaviors with a seed and a clock.
///
/// Feeding the same inputs, in order, to two stacks built the same way with the same seed gives the same poses
/// (the tests check this). Time is read from [`Inputs::time`]: the first tick starts the clock, and a tick that
/// goes back in time (a scrub) restarts every behavior's state.
#[derive(Debug, Clone)]
pub struct BehaviorStack {
    rig: Rig,
    slots: Vec<Slot>,
    seed: u64,
    start: Option<Tick>,
    last: Option<Tick>,
}

impl BehaviorStack {
    /// An empty stack for a rig.
    #[must_use]
    pub const fn new(rig: Rig) -> Self {
        Self { rig, slots: Vec::new(), seed: 0, start: None, last: None }
    }

    /// A stack with every behavior this crate has, with default parameters, in [`behaviors::IDS`] order.
    #[must_use]
    pub fn standard(rig: Rig) -> Self {
        let mut stack = Self::new(rig);
        for id in behaviors::IDS {
            if let Ok(b) = behaviors::create(id) {
                stack.slots.push(Slot { enabled: true, behavior: b });
            }
        }
        stack.reseed();
        stack
    }

    /// A stack from the behaviors bound to a puppet in a project.
    ///
    /// Each behavior starts from the stored `defaults` (the manifest's `tBehaviorDefaults`, if any) and takes the
    /// puppet's own parameter overrides on top. Behaviors this crate does not implement are skipped and their ids
    /// returned, so a tool can tell the user. The stack runs them in the order of [`behaviors::IDS`], not the
    /// file's, so physics always sees the head the face has just moved.
    #[must_use]
    pub fn from_bound(rig: Rig, bound: &[BoundBehavior], defaults: &[BehaviorDefaults]) -> (Self, Vec<String>) {
        let mut stack = Self::new(rig);
        let mut skipped = Vec::new();
        for b in bound {
            let mut pairs: Vec<(String, charcraft_ltab::LValue)> =
                defaults.iter().find(|d| d.behavior == b.behavior).map(|d| d.params.clone()).unwrap_or_default();
            pairs.extend(b.params.iter().cloned());
            match behaviors::create_from_pairs(&b.behavior, &pairs) {
                Ok(behavior) => stack.slots.push(Slot { enabled: true, behavior }),
                Err(_) => skipped.push(b.behavior.clone()),
            }
        }
        // Run in dependency order whatever order the file lists them in: the face and voice first, then the things
        // that react to them. The sort is stable, so two of a kind keep their order.
        let rank = |id: &str| behaviors::IDS.iter().position(|known| *known == id).unwrap_or(usize::MAX);
        stack.slots.sort_by_key(|s| rank(s.behavior.id()));
        stack.reseed();
        (stack, skipped)
    }

    /// Sets the seed of all random streams and restarts the stack.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self.reset();
        self
    }

    /// The seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Changes the seed and restarts the stack.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
        self.reset();
    }

    /// The rig.
    #[must_use]
    pub const fn rig(&self) -> &Rig {
        &self.rig
    }

    /// Replaces the rig (after the puppet was edited) and keeps behavior state.
    pub fn set_rig(&mut self, rig: Rig) {
        self.rig = rig;
    }

    /// The behaviors in order.
    #[must_use]
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// A slot to edit.
    pub fn slot_mut(&mut self, index: usize) -> Option<&mut Slot> {
        self.slots.get_mut(index)
    }

    /// Appends a behavior.
    pub fn push(&mut self, behavior: Box<dyn Behavior>) {
        self.insert(self.slots.len(), behavior);
    }

    /// Inserts a behavior at a position (the end when past it).
    pub fn insert(&mut self, index: usize, behavior: Box<dyn Behavior>) {
        let at = index.min(self.slots.len());
        self.slots.insert(at, Slot { enabled: true, behavior });
        self.reseed();
    }

    /// Removes a behavior.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSlot`].
    pub fn remove(&mut self, index: usize) -> Result<Slot> {
        if index >= self.slots.len() {
            return Err(Error::NoSuchSlot(index));
        }
        Ok(self.slots.remove(index))
    }

    /// Moves a behavior to another position.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSlot`].
    pub fn move_slot(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.slots.len() {
            return Err(Error::NoSuchSlot(from));
        }
        let slot = self.slots.remove(from);
        let at = to.min(self.slots.len());
        self.slots.insert(at, slot);
        self.reseed();
        Ok(())
    }

    /// Restarts every behavior and the clock.
    pub fn reset(&mut self) {
        self.start = None;
        self.last = None;
        self.reseed();
    }

    fn reseed(&mut self) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            slot.behavior.reset(self.seed.wrapping_add(u64::try_from(i).unwrap_or(0).wrapping_mul(0x9E37_79B9)));
        }
    }

    /// Runs every enabled behavior, in order, for one tick, and returns the pose they build together.
    pub fn step(&mut self, inputs: &Inputs) -> PuppetPose {
        let now = inputs.time;
        if self.last.is_some_and(|last| now < last) {
            self.reset();
        }
        let start = *self.start.get_or_insert(now);
        let dt = self.last.map_or(0.0, |last| Tick(now.0.saturating_sub(last.0)).seconds());
        let time = Tick(now.0.saturating_sub(start.0)).seconds();
        self.last = Some(now);

        let mut pose = PuppetPose::neutral();
        let cx = Cx { inputs, rig: &self.rig, time, dt };
        for slot in self.slots.iter_mut().filter(|s| s.enabled) {
            slot.behavior.step(&cx, &mut pose);
        }
        prune_identity(&mut pose);
        pose.time_millis = u64::try_from(anim_core::num::round_f64_to_i64(time * 1000.0)).unwrap_or(0);
        pose
    }

    /// Runs a sequence of ticks and returns each pose.
    pub fn run<'a>(&mut self, inputs: impl IntoIterator<Item = &'a Inputs>) -> Vec<PuppetPose> {
        inputs.into_iter().map(|i| self.step(i)).collect()
    }
}

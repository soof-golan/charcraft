//! The integration tests of `charcraft-behaviors`, built as one test binary (one link instead of five). Each module is
//! a theme; shared helpers live in `common`.

mod common;

mod behaviors;
mod real_poses;
mod robust;
mod samples;
mod view_behaviors;
mod visual_body;
mod visual_face;

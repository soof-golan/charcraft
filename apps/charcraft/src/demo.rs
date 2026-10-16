//! The built-in demo puppet: the classic character of the Puppet Maker.
//!
//! A round-faced character with the tags Character Animator reads (`Head`, `Left Eye`, `+ Left Pupil`,
//! `Left Blink`, eyebrows, a `mouth` group with all fourteen mouth shapes, a `Body`). It lets `--smoke`,
//! `--screenshot` and a first launch work without any artwork on disk.

use anim_artwork::ArtworkTree;
use charcraft_maker::Spec;
use charcraft_puppet::Puppet;

/// The demo character's artwork.
#[must_use]
pub fn artwork() -> ArtworkTree {
    charcraft_maker::artwork(&Spec::default())
}

/// The demo puppet.
///
/// # Errors
/// Whatever building a puppet from the artwork reports (it cannot fail for this artwork).
pub fn puppet() -> charcraft_puppet::Result<Puppet> {
    charcraft_maker::puppet(&Spec::default())
}

#[cfg(test)]
mod tests {
    use charcraft_puppet::{PuppetPose, Viseme};

    use super::*;

    #[test]
    fn the_demo_puppet_has_a_rig() {
        let p = puppet().unwrap();
        assert_eq!(p.mouth_groups().len(), 1);
        assert_eq!(p.mouth_shapes(p.mouth_groups()[0]).len(), 14);
        assert!(p.find_path("Head/Left Eye").is_some());
        assert!(p.find_path("Head/Right Eye/Right Blink").is_some());
        let list = p.evaluate(&PuppetPose::neutral().with_viseme(Viseme::Ah));
        assert!(!list.nodes.is_empty());
    }
}

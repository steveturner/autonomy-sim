use crate::model::{Affiliation, EntityKind};

/// MIL-STD-2525C position 4. All simulated entities currently exist in the
/// scenario; `Planned` is exposed for scenario builders that render waypoints
/// or future sites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SymbolStatus {
    #[default]
    Present,
    Planned,
}

/// Returns a 15-character MIL-STD-2525C Symbol Identification Code (SIDC).
///
/// The mappings use the 2525C warfighting, tactical-graphics, and emergency-
/// management coding schemes as appropriate. Position 2 carries affiliation,
/// position 3 battle dimension/category, position 4 status, positions 5-10 the
/// function identifier, and positions 11-15 the symbol modifiers.
pub fn sidc(kind: EntityKind, affiliation: Affiliation, status: SymbolStatus) -> String {
    let identity = match affiliation {
        Affiliation::Friendly => 'F',
        Affiliation::Hostile => 'H',
        Affiliation::Neutral => 'N',
        Affiliation::Unknown => 'U',
    };
    let present = match status {
        SymbolStatus::Present => 'P',
        SymbolStatus::Planned => 'A',
    };
    let (scheme, dimension, function, modifier) = match kind {
        // MFQ is the 2525C fixed-wing remotely piloted vehicle/UAV function.
        EntityKind::Uas | EntityKind::AirTanker | EntityKind::ThreatUas => {
            ('S', 'A', "MFQ---", "-----")
        }
        EntityKind::Rotary => ('S', 'A', "MH----", "-----"),
        EntityKind::Interceptor => ('S', 'A', "MFFI--", "-----"),
        EntityKind::Person => ('S', 'G', "U-----", "A----"),
        EntityKind::GroundVehicle => ('S', 'G', "EVU---", "-----"),
        EntityKind::Base => ('S', 'G', "IB----", "H----"),
        EntityKind::ProtectedSite => ('S', 'G', "I-----", "H----"),
        EntityKind::RadarSensor => ('S', 'G', "ESR---", "-----"),
        EntityKind::EwJammer => ('S', 'G', "UUMSEJ", "-----"),
        EntityKind::GunSystem => ('S', 'G', "EWD---", "-----"),
        // 2525C Appendix G: Emergency Management / Incident / Fire / Wild Fire.
        EntityKind::Fire => ('E', 'I', "CH----", "-----"),
        // 2525C tactical graphic: Action Point / Waypoint.
        EntityKind::Waypoint => ('G', 'G', "GPP---", "-----"),
    };
    format!("{scheme}{identity}{dimension}{present}{function}{modifier}")
}

pub fn icon_hint(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Uas | EntityKind::ThreatUas => "fixed_wing_uas",
        EntityKind::AirTanker => "air_tanker",
        EntityKind::Rotary => "rotary_wing",
        EntityKind::Person => "person",
        EntityKind::GroundVehicle => "ground_vehicle",
        EntityKind::Base => "airfield",
        EntityKind::Fire => "wildfire",
        EntityKind::Waypoint => "waypoint",
        EntityKind::RadarSensor => "radar",
        EntityKind::EwJammer => "ew_jammer",
        EntityKind::Interceptor => "interceptor",
        EntityKind::GunSystem => "gun_system",
        EntityKind::ProtectedSite => "protected_site",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidcs_are_2525c_length_and_encode_affiliation() {
        let kinds = [
            EntityKind::Uas,
            EntityKind::AirTanker,
            EntityKind::Rotary,
            EntityKind::Person,
            EntityKind::GroundVehicle,
            EntityKind::Base,
            EntityKind::Fire,
            EntityKind::Waypoint,
            EntityKind::ThreatUas,
            EntityKind::RadarSensor,
            EntityKind::EwJammer,
            EntityKind::Interceptor,
            EntityKind::GunSystem,
            EntityKind::ProtectedSite,
        ];
        for kind in kinds {
            let friendly = sidc(kind, Affiliation::Friendly, SymbolStatus::Present);
            let hostile = sidc(kind, Affiliation::Hostile, SymbolStatus::Present);
            assert_eq!(friendly.len(), 15, "{kind:?}: {friendly}");
            assert_eq!(friendly.as_bytes()[1], b'F');
            assert_eq!(hostile.as_bytes()[1], b'H');
        }
        assert_eq!(
            sidc(
                EntityKind::Uas,
                Affiliation::Friendly,
                SymbolStatus::Present
            ),
            "SFAPMFQ--------"
        );
        assert_eq!(
            sidc(
                EntityKind::ThreatUas,
                Affiliation::Hostile,
                SymbolStatus::Present
            ),
            "SHAPMFQ--------"
        );
        assert_eq!(
            sidc(
                EntityKind::Fire,
                Affiliation::Friendly,
                SymbolStatus::Present
            ),
            "EFIPCH---------"
        );
        assert_eq!(
            sidc(
                EntityKind::Base,
                Affiliation::Friendly,
                SymbolStatus::Present
            ),
            "SFGPIB----H----"
        );
    }
}

//! Core domain types for arm-core: joints, software limits, and the safety
//! guard that refuses out-of-range targets before they reach the controller.

use std::fmt;

/// Number of controllable joints on the Dorna 1 (5-axis).
pub const JOINT_COUNT: usize = 5;

/// A joint-space position: one angle in degrees per joint, j0..j4.
pub type Joints = [f64; JOINT_COUNT];

/// Software joint limits in degrees, from the arm's factory calibration
/// (the dorna `config.yaml` `limit` block). `None` means the joint is
/// unconstrained in software: the wrist joints j3/j4 rotate freely.
pub const JOINT_LIMITS: [Option<(f64, f64)>; JOINT_COUNT] = [
    Some((-175.0, 175.0)), // j0 base
    Some((-175.0, 160.0)), // j1 lower arm
    Some((-130.0, 130.0)), // j2 upper arm
    None,                  // j3 wrist
    None,                  // j4 wrist
];

/// A requested joint angle that falls outside its software limit.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitViolation {
    pub joint: usize,
    pub requested: f64,
    pub min: f64,
    pub max: f64,
}

impl fmt::Display for LimitViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "joint j{} = {:.4} deg is outside software limit [{:.1}, {:.1}]",
            self.joint, self.requested, self.min, self.max
        )
    }
}

impl std::error::Error for LimitViolation {}

/// Check a full joint-space target against the software limits.
///
/// Returns the first violation found (lowest joint index), or `Ok(())` if every
/// constrained joint is within range. Limits are inclusive at the boundary.
/// This is the guard that must run before any move command is sent to the arm.
pub fn check_joint_limits(target: &Joints) -> Result<(), LimitViolation> {
    for (joint, &requested) in target.iter().enumerate() {
        if let Some((min, max)) = JOINT_LIMITS[joint] {
            if requested < min || requested > max {
                return Err(LimitViolation { joint, requested, min, max });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The factory calibration home pose must itself be inside the limits,
    /// otherwise a freshly homed arm would already be "in violation".
    #[test]
    fn factory_home_pose_is_within_limits() {
        let home: Joints = [-8.2671, 139.7344, -87.1807, 220.2257, -65.439];
        assert!(check_joint_limits(&home).is_ok());
    }

    #[test]
    fn neutral_zero_pose_is_ok() {
        assert!(check_joint_limits(&[0.0; JOINT_COUNT]).is_ok());
    }

    #[test]
    fn rejects_j0_over_max() {
        let err = check_joint_limits(&[200.0, 0.0, 0.0, 0.0, 0.0]).unwrap_err();
        assert_eq!(err.joint, 0);
        assert_eq!(err.max, 175.0);
        assert_eq!(err.requested, 200.0);
    }

    #[test]
    fn rejects_j1_over_asymmetric_max() {
        // j1 range is asymmetric: [-175, 160]. 165 is past the top only.
        let err = check_joint_limits(&[0.0, 165.0, 0.0, 0.0, 0.0]).unwrap_err();
        assert_eq!(err.joint, 1);
        assert_eq!(err.max, 160.0);
        // ...but -170 is still legal at the bottom.
        assert!(check_joint_limits(&[0.0, -170.0, 0.0, 0.0, 0.0]).is_ok());
    }

    #[test]
    fn rejects_j2_under_min() {
        let err = check_joint_limits(&[0.0, 0.0, -130.1, 0.0, 0.0]).unwrap_err();
        assert_eq!(err.joint, 2);
        assert_eq!(err.min, -130.0);
    }

    #[test]
    fn wrist_joints_are_unconstrained() {
        // j3/j4 have no software limit; huge angles are accepted.
        assert!(check_joint_limits(&[0.0, 0.0, 0.0, 100_000.0, -99_999.0]).is_ok());
    }

    #[test]
    fn boundaries_are_inclusive() {
        assert!(check_joint_limits(&[175.0, 160.0, 130.0, 0.0, 0.0]).is_ok());
        assert!(check_joint_limits(&[-175.0, -175.0, -130.0, 0.0, 0.0]).is_ok());
    }

    #[test]
    fn just_past_boundary_is_rejected() {
        assert!(check_joint_limits(&[175.0001, 0.0, 0.0, 0.0, 0.0]).is_err());
        assert!(check_joint_limits(&[-175.0001, 0.0, 0.0, 0.0, 0.0]).is_err());
    }

    #[test]
    fn reports_lowest_index_violation_first() {
        // Both j0 and j1 are out of range; j0 (lower index) is reported.
        let err = check_joint_limits(&[200.0, 200.0, 0.0, 0.0, 0.0]).unwrap_err();
        assert_eq!(err.joint, 0);
    }

    #[test]
    fn violation_display_is_readable() {
        let err = check_joint_limits(&[0.0, 0.0, 200.0, 0.0, 0.0]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("j2"));
        assert!(msg.contains("130"));
    }
}

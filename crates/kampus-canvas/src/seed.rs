//! The versioned synthetic seed (`fixtures/canvas-synthetic/seed.json`).

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::CanvasError;

const BUNDLED: &str = include_str!("../../../fixtures/canvas-synthetic/seed.json");

/// Server-controlled synthetic course-code enumeration. The client rejects
/// any upstream course code outside it, so output labels never carry
/// upstream text.
pub const COURSE_CODES: &[&str] = &["SYN-1010", "SYN-2020", "SYN-3030", "SYN-4040"];

/// Seed offsets are bounded so anchoring them can never overflow.
const MAX_OFFSET_MINUTES: i64 = 366 * 24 * 60;

/// Parsed and cross-checked synthetic seed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    synthetic: bool,
    version: u32,
    #[allow(dead_code)]
    provenance: String,
    pub(crate) users: Vec<SeedUser>,
    pub(crate) courses: Vec<SeedCourse>,
    pub(crate) enrollments: Vec<SeedEnrollment>,
    pub(crate) assignments: Vec<SeedAssignment>,
    #[serde(default)]
    pub(crate) announcements: Vec<SeedAnnouncement>,
    #[serde(default)]
    pub(crate) submissions: Vec<SeedSubmission>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeedUser {
    pub(crate) canvas_user_id: u64,
    pub(crate) synthetic_user_id: String,
    pub(crate) kampus_subjects: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeedCourse {
    pub(crate) id: u64,
    pub(crate) course_code: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeedEnrollment {
    pub(crate) user_id: u64,
    pub(crate) course_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeedAssignment {
    pub(crate) id: u64,
    pub(crate) course_id: u64,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) due_offset_minutes: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeedAnnouncement {
    pub(crate) id: u64,
    pub(crate) course_id: u64,
    pub(crate) title: String,
    pub(crate) posted_offset_minutes: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeedSubmission {
    pub(crate) user_id: u64,
    pub(crate) assignment_id: u64,
    pub(crate) workflow_state: String,
}

impl Seed {
    /// The seed compiled into sandbox builds.
    pub fn bundled() -> Result<Self, CanvasError> {
        Self::parse(BUNDLED)
    }

    /// Parse and cross-check a seed document.
    pub fn parse(json: &str) -> Result<Self, CanvasError> {
        let seed: Self = serde_json::from_str(json)
            .map_err(|_| CanvasError::Seed("invalid seed document".to_owned()))?;
        seed.check().map_err(|m| CanvasError::Seed(m.to_owned()))?;
        Ok(seed)
    }

    fn check(&self) -> Result<(), &'static str> {
        if !self.synthetic {
            return Err("seed must be marked synthetic");
        }
        if self.version != 1 {
            return Err("unsupported seed version");
        }
        let users = unique(self.users.iter().map(|u| u.canvas_user_id))
            .ok_or("duplicate canvas_user_id")?;
        unique(self.users.iter().map(|u| u.synthetic_user_id.as_str()))
            .ok_or("duplicate synthetic_user_id")?;
        unique(self.users.iter().flat_map(|u| &u.kampus_subjects))
            .ok_or("a kampus subject maps to more than one synthetic user")?;
        let courses = unique(self.courses.iter().map(|c| c.id)).ok_or("duplicate course id")?;
        if self
            .courses
            .iter()
            .any(|c| !COURSE_CODES.contains(&c.course_code.as_str()))
        {
            return Err("course_code outside the synthetic enumeration");
        }
        let assignments =
            unique(self.assignments.iter().map(|a| a.id)).ok_or("duplicate assignment id")?;
        let mut offsets = self
            .assignments
            .iter()
            .map(|a| a.due_offset_minutes)
            .chain(self.announcements.iter().map(|a| a.posted_offset_minutes));
        if offsets.any(|m| !(-MAX_OFFSET_MINUTES..=MAX_OFFSET_MINUTES).contains(&m)) {
            return Err("time offsets must stay within a year of the anchor");
        }
        let known_course = |id| courses.contains(&id);
        if !self
            .enrollments
            .iter()
            .all(|e| users.contains(&e.user_id) && known_course(e.course_id))
            || !self.assignments.iter().all(|a| known_course(a.course_id))
            || !self.announcements.iter().all(|a| known_course(a.course_id))
            || !self
                .submissions
                .iter()
                .all(|s| users.contains(&s.user_id) && assignments.contains(&s.assignment_id))
        {
            return Err("dangling user, course or assignment reference");
        }
        Ok(())
    }

    /// Course IDs the Canvas user is enrolled in.
    pub(crate) fn courses_of(&self, canvas_user_id: u64) -> BTreeSet<u64> {
        self.enrollments
            .iter()
            .filter(|e| e.user_id == canvas_user_id)
            .map(|e| e.course_id)
            .collect()
    }
}

/// The set of items, or `None` if any repeats.
fn unique<T: Ord>(items: impl Iterator<Item = T>) -> Option<BTreeSet<T>> {
    let mut set = BTreeSet::new();
    for item in items {
        if !set.insert(item) {
            return None;
        }
    }
    Some(set)
}

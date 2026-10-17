//! Context-pack manifest types.
//!
//! Derived from openrig packages/daemon/src/domain/context-packs/context-pack-types.ts
//! (v0.6.9, Apache-2.0); ported to Rust and modified.

/// Founder taxonomy shared by packs and atoms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Taxonomy {
    /// Where you are.
    World,
    /// What has been learned here.
    Lore,
    /// What you know how to do.
    Skills,
    /// What you are doing now.
    Mission,
}

/// Situation tag used by the composition algebra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Situation {
    /// Base walk.
    Fresh,
    /// Handover material.
    Handover,
    /// Post-compaction subset.
    PostCompaction,
}

/// Depth or width of an atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Purpose {
    /// Deep material.
    Depth,
    /// Wide material.
    Width,
}

/// Runtime an atom applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AtomRuntime {
    /// Claude only.
    Claude,
    /// Codex only.
    Codex,
    /// Both.
    Any,
}

/// Drop order when a budget binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Drop first.
    Optional,
    /// Drop after optional.
    Recommended,
    /// Drop last.
    Core,
}

/// Named-profile runtime. `any` is not a profile runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProfileRuntime {
    /// Claude.
    Claude,
    /// Codex.
    Codex,
}

/// Situated context source a profile phase may insert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProfileSource {
    /// Project tree.
    Project,
    /// Mission tree.
    Mission,
    /// Seat tree.
    Seat,
    /// Slice walk.
    Slice,
}

/// World-anatomy tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Region {
    /// Identity.
    Identity,
    /// Ontology.
    Ontology,
    /// Terrain.
    Terrain,
    /// Actors.
    Actors,
    /// Laws.
    Laws,
    /// History.
    History,
    /// State.
    State,
    /// Affordances.
    Affordances,
}

/// Teaching sentence appended to a missing or invalid taxonomy.
pub const TAXONOMY_TEACHING: &str = "Legal values: world | lore | skills | mission. \
world = where you are (the environment: entities, rules, affordances); \
lore = what has been learned here (position knowledge earned in place); \
skills = what you know how to do (procedural capability); \
mission = what you are doing now (the current work and its intent). \
Add one line to manifest.yaml, e.g. `taxonomy: skills`.";

/// One file declared by a manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackFile {
    /// Path relative to the pack directory.
    pub path: String,
    /// Operator role. Required by the TypeScript schema.
    pub role: String,
    /// One-line description, when present.
    pub summary: Option<String>,
}

/// Acceptance probe on an atom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// Natural prompt.
    pub prompt: String,
    /// Expected observable behavior.
    pub expect: String,
    /// Compilable regex sources, when present.
    pub expected_patterns: Option<Vec<String>>,
    /// Judged rubric, when present.
    pub rubric: Option<String>,
}

/// An install atom: an address plus composition metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atom {
    /// Stable slug.
    pub id: String,
    /// `file` or `file#H2/H3`, or a tree-prefixed ref.
    pub address: String,
    /// Atom taxonomy.
    pub taxonomy: Taxonomy,
    /// World-anatomy tags.
    pub regions: Option<Vec<Region>>,
    /// Situation selector. Never empty.
    pub situations: Vec<Situation>,
    /// Depth or width.
    pub purpose: Purpose,
    /// Runtime filter. Defaults to [`AtomRuntime::Any`].
    pub runtime: AtomRuntime,
    /// Walk position.
    pub order: i64,
    /// Dependency ids.
    pub requires: Option<Vec<String>>,
    /// Budget drop tier.
    pub priority: Priority,
    /// Named-profile-only atoms skip the legacy situation walk.
    pub profile_only: bool,
    /// Optional acceptance probe.
    pub probe: Option<Probe>,
}

/// One phase of a named profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfilePhase {
    /// Stable slug.
    pub id: String,
    /// Atom ids, when this phase selects atoms.
    pub atoms: Option<Vec<String>>,
    /// Context sources, when this phase inserts situated context.
    pub context: Option<Vec<ProfileSource>>,
}

/// A named selection over one pack's atom graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Stable slug.
    pub id: String,
    /// Situations the profile applies to.
    pub situations: Vec<Situation>,
    /// Runtimes the profile applies to.
    pub runtimes: Vec<ProfileRuntime>,
    /// Ordered phases.
    pub phases: Vec<ProfilePhase>,
}

/// A parsed context-pack manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackManifest {
    /// Pack name.
    pub name: String,
    /// Version token.
    pub version: String,
    /// Operator purpose. The slice sketch calls this the description.
    pub description: Option<String>,
    /// Pack-level taxonomy.
    pub taxonomy: Taxonomy,
    /// Declared files.
    pub files: Vec<PackFile>,
    /// Operator token hint.
    pub estimated_tokens: Option<u32>,
    /// Install atoms.
    pub atoms: Vec<Atom>,
    /// Named profiles.
    pub profiles: Vec<Profile>,
}

impl PackManifest {
    /// YAML `purpose`, when set.
    #[must_use]
    pub fn purpose(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

macro_rules! token_enum {
    ($ty:ty, $($name:ident => $text:literal),+ $(,)?) => {
        impl $ty {
            /// Parse the TypeScript literal.
            #[must_use]
            pub fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$name),)+
                    _ => None,
                }
            }

            /// TypeScript literal.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$name => $text,)+
                }
            }
        }
    };
}

token_enum!(
    Taxonomy,
    World => "world",
    Lore => "lore",
    Skills => "skills",
    Mission => "mission",
);
token_enum!(
    Situation,
    Fresh => "fresh",
    Handover => "handover",
    PostCompaction => "post-compaction",
);
token_enum!(Purpose, Depth => "depth", Width => "width");
token_enum!(AtomRuntime, Claude => "claude", Codex => "codex", Any => "any");
token_enum!(
    Priority,
    Optional => "optional",
    Recommended => "recommended",
    Core => "core",
);
token_enum!(ProfileRuntime, Claude => "claude", Codex => "codex");
token_enum!(
    ProfileSource,
    Project => "project",
    Mission => "mission",
    Seat => "seat",
    Slice => "slice",
);
token_enum!(
    Region,
    Identity => "identity",
    Ontology => "ontology",
    Terrain => "terrain",
    Actors => "actors",
    Laws => "laws",
    History => "history",
    State => "state",
    Affordances => "affordances",
);

/// Allowed file suffixes, in the TypeScript order.
pub const FILE_SUFFIXES: &[&str] = &[
    ".md",
    ".markdown",
    ".yaml",
    ".yml",
    ".txt",
    ".sh",
    ".ts",
    ".mjs",
    ".py",
];

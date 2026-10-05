//! Durable, resumable proof of one live OCI graph for incremental GC.
mod engine;

use anyhow::{Result, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const METADATA_BYTES_PER_CALL: u64 = 8 << 20;
const WORK_ITEMS_PER_CALL: usize = 512;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum GraphKind {
    Index,
    Manifest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum DescriptorTarget {
    Image {
        kind: GraphKind,
        platform: Option<Platform>,
        depth: u8,
    },
    Config {
        manifest: u16,
        layer_count: u32,
        platform: Option<Platform>,
    },
    Layer,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DescriptorTask {
    pub(crate) parent: u16,
    pub(crate) child: u16,
    pub(crate) size: u64,
    pub(crate) target: DescriptorTarget,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DescriptorClass {
    Image,
    Config,
    Layer,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct DescriptorKey {
    parent: u16,
    child: u16,
    class: DescriptorClass,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DescriptorRecord {
    key: DescriptorKey,
    size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Task {
    ProbeRoot {
        node: u16,
    },
    Document {
        node: u16,
        expected: Option<GraphKind>,
        platform: Option<Platform>,
        depth: u8,
        root: bool,
    },
    Descriptor(DescriptorTask),
    Config {
        node: u16,
        manifest: u16,
        layer_count: u32,
        platform: Option<Platform>,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum WalkPhase {
    Discover,
    Edges,
    Images,
    Complete,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ImagePlatform {
    pub(crate) manifest: u16,
    pub(crate) platform: Platform,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GcWalk {
    pub(crate) key: String,
    pub(crate) digest: Option<ArtifactDigest>,
    pub(crate) handles: bool,
    pub(crate) catalog: Vec<ArtifactDigest>,
    pub(crate) pending: Vec<Task>,
    scheduled: BTreeSet<u16>,
    expanded: BTreeSet<u16>,
    pub(crate) edges: BTreeSet<(u16, u16)>,
    descriptors: Vec<DescriptorRecord>,
    pub(crate) images: Vec<ImagePlatform>,
    pub(crate) root_kind: Option<GraphKind>,
    phase: WalkPhase,
    cursor: usize,
    pub(crate) raw: bool,
}

pub(super) struct Budget {
    bytes: u64,
    items: usize,
}

impl Budget {
    pub(super) fn new() -> Self {
        Self {
            bytes: METADATA_BYTES_PER_CALL,
            items: WORK_ITEMS_PER_CALL,
        }
    }

    fn document(&mut self, size: u64, max_metadata: u64) -> Result<bool> {
        ensure!(size <= max_metadata, "metadata exceeds limit");
        let cost = size
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("GC metadata work overflow"))?;
        if self.items == 0 || cost > self.bytes {
            return Ok(false);
        }
        self.bytes -= cost;
        self.items -= 1;
        Ok(true)
    }

    fn item(&mut self) -> bool {
        if self.items == 0 {
            return false;
        }
        self.items -= 1;
        true
    }

    pub(super) fn marks(&mut self, count: usize) -> bool {
        if count > self.items {
            return false;
        }
        self.items -= count;
        true
    }
}

impl GcWalk {
    pub(super) fn new(key: String, digest: Option<ArtifactDigest>, handles: bool) -> Self {
        let mut walk = Self {
            key,
            digest,
            handles,
            catalog: Vec::new(),
            pending: Vec::new(),
            scheduled: BTreeSet::new(),
            expanded: BTreeSet::new(),
            edges: BTreeSet::new(),
            descriptors: Vec::new(),
            images: Vec::new(),
            root_kind: None,
            phase: WalkPhase::Discover,
            cursor: 0,
            raw: false,
        };
        if let Some(digest) = walk.digest.clone() {
            walk.catalog.push(digest);
            walk.pending.push(Task::ProbeRoot { node: 0 });
        } else {
            walk.phase = WalkPhase::Edges;
        }
        walk
    }

    pub(crate) fn intern(&mut self, digest: ArtifactDigest, max_graph: usize) -> Result<u16> {
        if let Some(index) = self.catalog.iter().position(|item| item == &digest) {
            return u16::try_from(index).map_err(Into::into);
        }
        ensure!(
            self.catalog.len() < max_graph.saturating_add(1),
            "OCI graph nodes exceed bound"
        );
        let index = u16::try_from(self.catalog.len())?;
        self.catalog.push(digest);
        Ok(index)
    }

    pub(crate) fn push(&mut self, task: Task) {
        self.pending.push(task);
    }

    pub(crate) fn schedule_image(&mut self, node: u16, max_graph: usize) -> Result<()> {
        ensure!(self.scheduled.len() < max_graph, "OCI graph limit exceeded");
        ensure!(
            self.scheduled.insert(node),
            "OCI graph cycle or duplicate manifest"
        );
        Ok(())
    }

    pub(crate) fn mark_expanded(&mut self, node: u16) -> Result<()> {
        ensure!(
            self.scheduled.contains(&node) && self.expanded.insert(node),
            "OCI graph cycle or duplicate manifest"
        );
        Ok(())
    }

    pub(crate) fn add_descriptor(
        &mut self,
        parent: u16,
        digest: ArtifactDigest,
        size: u64,
        target: DescriptorTarget,
        max_graph: usize,
    ) -> Result<()> {
        let child = self.intern(digest, max_graph)?;
        let class = match &target {
            DescriptorTarget::Image { .. } => DescriptorClass::Image,
            DescriptorTarget::Config { .. } => DescriptorClass::Config,
            DescriptorTarget::Layer => DescriptorClass::Layer,
        };
        self.edges.insert((parent, child));
        ensure!(self.edges.len() <= max_graph, "OCI graph limit exceeded");
        if class == DescriptorClass::Image {
            self.schedule_image(child, max_graph)?;
        }
        let key = DescriptorKey {
            parent,
            child,
            class,
        };
        if let Some(previous) = self.descriptors.iter().find(|item| item.key == key) {
            ensure!(previous.size == size, "descriptor size mismatch");
            return Ok(());
        }
        ensure!(
            self.descriptors.len() < max_graph.saturating_mul(2),
            "OCI graph descriptor work exceeds bound"
        );
        self.descriptors.push(DescriptorRecord { key, size });
        self.pending.push(Task::Descriptor(DescriptorTask {
            parent,
            child,
            size,
            target,
        }));
        Ok(())
    }

    fn complete(&self) -> bool {
        self.phase == WalkPhase::Complete
    }

    fn nodes(&self) -> Vec<ArtifactDigest> {
        self.catalog.clone()
    }
}

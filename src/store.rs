use crate::model::{Index, INDEX_VERSION};
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

pub fn default_path(root: &Path) -> PathBuf {
    root.join(".connectome").join("index.bin")
}

pub fn save(index: &Index, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let encoded = bincode::serialize(index)?;
    let temporary = path.with_extension("bin.tmp");
    std::fs::write(&temporary, encoded)
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("cannot publish {}", path.display()))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Index> {
    let bytes = std::fs::read(path).with_context(|| {
        format!(
            "cannot read index {}; run index_repository first",
            path.display()
        )
    })?;
    let index: Index = bincode::deserialize(&bytes).context("invalid connectome index")?;
    if index.version != INDEX_VERSION {
        return Err(anyhow!(
            "index version {} is unsupported; rebuild it",
            index.version
        ));
    }
    Ok(index)
}

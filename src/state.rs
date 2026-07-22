use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Sidecar run-state in `~/.goulash/state.toml` — goulash's scratch,
/// never the user's config. Powers the model **crash fuse**
/// (wiki: interaction/settings-and-nav.md): a too-big model can OOM the
/// machine while loading, and if it's the persisted default every boot
/// walks into the same wall. So trust is earned: an in-flight mark that
/// survives to the next boot means that run died with the model on the
/// hook, and the model is never auto-bound again until an explicit retry
/// survives a generation.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StateFile {
    /// Set while a load/generation is in flight — the dangerous window.
    loading: Option<String>,
    /// Persisted default that hasn't survived a generation yet.
    pub probation: Option<String>,
    /// Tripped fuse: never auto-bound; explicit retry only.
    pub distrusted: Option<String>,
    /// Last model that completed a generation — the fuse's safe landing.
    pub last_good: Option<String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl StateFile {
    pub fn load(dir: Option<PathBuf>) -> StateFile {
        let path = dir.map(|d| d.join("state.toml"));
        let mut s: StateFile = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default();
        s.path = path;
        // An in-flight mark surviving to a fresh boot = unclean death.
        if let Some(m) = s.loading.take() {
            s.distrusted = Some(m);
            s.save();
        }
        s
    }

    fn save(&self) {
        if let Some(p) = &self.path {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = toml::to_string(self) {
                let _ = std::fs::write(p, text);
            }
        }
    }

    pub fn busy(&mut self, model: &str) {
        if self.loading.as_deref() != Some(model) {
            self.loading = Some(model.to_string());
            self.save();
        }
    }

    pub fn idle(&mut self) {
        if self.loading.take().is_some() {
            self.save();
        }
    }

    /// A generation completed on `model`: it demonstrably survived.
    /// Promote to last_good; end its probation and clear its distrust.
    pub fn promote(&mut self, model: &str) {
        let mut changed = self.loading.take().is_some();
        if self.probation.as_deref() == Some(model) {
            self.probation = None;
            changed = true;
        }
        if self.distrusted.as_deref() == Some(model) {
            self.distrusted = None;
            changed = true;
        }
        if self.last_good.as_deref() != Some(model) {
            self.last_good = Some(model.to_string());
            changed = true;
        }
        if changed {
            self.save();
        }
    }

    /// A model was persisted as the default: unproven until a generation
    /// completes. An explicit save is also an explicit retry, so any
    /// standing distrust of it is cleared.
    pub fn set_probation(&mut self, model: &str) {
        self.probation = Some(model.to_string());
        if self.distrusted.as_deref() == Some(model) {
            self.distrusted = None;
        }
        self.save();
    }

    /// Startup verdict on the configured default. `Some(fallback)` means
    /// refuse to auto-bind it and land on the fallback (`None` = auto).
    pub fn veto(&self, configured: Option<&str>) -> Option<Option<String>> {
        match (configured, self.distrusted.as_deref()) {
            (Some(c), Some(d)) if c == d => Some(self.last_good.clone().filter(|g| g != c)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StateFile;

    #[test]
    fn fuse_trips_on_unclean_death_and_clears_on_survival() {
        let dir = std::env::temp_dir().join(format!("goulash-fuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Session 1: generation starts on "big" and the process dies.
        let mut s = StateFile::load(Some(dir.clone()));
        s.busy("big");

        // Session 2: the leftover mark trips the fuse.
        let s2 = StateFile::load(Some(dir.clone()));
        assert_eq!(s2.distrusted.as_deref(), Some("big"));
        assert_eq!(s2.veto(Some("big")), Some(None)); // no last_good yet
        assert_eq!(s2.veto(Some("small")), None);

        // Explicit retry survives: promote clears everything.
        let mut s3 = StateFile::load(Some(dir.clone()));
        s3.busy("big");
        s3.promote("big");
        assert!(s3.distrusted.is_none() && s3.probation.is_none());
        assert_eq!(s3.last_good.as_deref(), Some("big"));

        // Session 4: clean marks, no veto; last_good is the landing spot
        // if some OTHER model trips later.
        let mut s4 = StateFile::load(Some(dir.clone()));
        assert_eq!(s4.veto(Some("big")), None);
        s4.busy("huge");
        let s5 = StateFile::load(Some(dir.clone()));
        assert_eq!(s5.veto(Some("huge")), Some(Some("big".to_string())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_idle_leaves_no_mark() {
        let dir = std::env::temp_dir().join(format!("goulash-fuse2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = StateFile::load(Some(dir.clone()));
        s.busy("m");
        s.idle();
        let s2 = StateFile::load(Some(dir.clone()));
        assert!(s2.distrusted.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_is_probation_and_explicit_retry() {
        let mut s = StateFile {
            distrusted: Some("big".to_string()),
            ..Default::default()
        };
        s.set_probation("big");
        assert!(s.distrusted.is_none());
        assert_eq!(s.probation.as_deref(), Some("big"));
    }
}

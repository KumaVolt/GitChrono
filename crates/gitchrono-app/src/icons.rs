//! Toolbar and file-list icons. Most are Lucide icons from gpui-kit's
//! catalog, embedded on demand; the stash trays are drawn here in the same style.

use std::borrow::Cow;

use gpui_kit::assets::{IconName, IconNamed};
use gpui_kit::*;

gpui_kit::assets::icon_assets!(Lucide, [FolderOpen, RotateCcw, RotateCw, RefreshCw, ArrowDownToLine, ArrowUpFromLine, GitBranch, Terminal, Sparkles, List, ListTree, ListPlus, ListMinus]);

const STASH: (&str, &[u8]) = ("icons/gitchrono/stash.svg", include_bytes!("../assets/icons/stash.svg"));
const STASH_POP: (&str, &[u8]) = ("icons/gitchrono/stash-pop.svg", include_bytes!("../assets/icons/stash-pop.svg"));

#[derive(Clone, Copy)]
pub enum ToolIcon {
    Open,
    Undo,
    Redo,
    Fetch,
    Pull,
    Push,
    Branch,
    Stash,
    Pop,
    Terminal,
    Ai,
}

impl IconNamed for ToolIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Open => IconName::FolderOpen.path(),
            Self::Undo => IconName::RotateCcw.path(),
            Self::Redo => IconName::RotateCw.path(),
            Self::Fetch => IconName::RefreshCw.path(),
            Self::Pull => IconName::ArrowDownToLine.path(),
            Self::Push => IconName::ArrowUpFromLine.path(),
            Self::Branch => IconName::GitBranch.path(),
            Self::Stash => STASH.0.into(),
            Self::Pop => STASH_POP.0.into(),
            Self::Terminal => IconName::Terminal.path(),
            Self::Ai => IconName::Sparkles.path(),
        }
    }
}

/// The app's asset source: just the icons above.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match [STASH, STASH_POP].into_iter().find(|(p, _)| *p == path) {
            Some((_, svg)) => Ok(Some(Cow::Borrowed(svg))),
            None => Lucide.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Lucide.list(path)?;
        paths.extend([STASH.0, STASH_POP.0].into_iter().filter(|p| p.starts_with(path)).map(SharedString::from));
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's `test` macro would shadow `#[test]`.
    use super::{Assets, ToolIcon};
    use gpui_kit::AssetSource as _;
    use gpui_kit::assets::IconNamed as _;

    const ALL: [ToolIcon; 11] = [
        ToolIcon::Open,
        ToolIcon::Undo,
        ToolIcon::Redo,
        ToolIcon::Fetch,
        ToolIcon::Pull,
        ToolIcon::Push,
        ToolIcon::Branch,
        ToolIcon::Stash,
        ToolIcon::Pop,
        ToolIcon::Terminal,
        ToolIcon::Ai,
    ];

    #[test]
    fn every_toolbar_icon_is_embedded() {
        for icon in ALL {
            let path = icon.path();
            let svg = Assets.load(&path).unwrap().unwrap_or_else(|| panic!("{path} is not embedded"));
            assert!(svg.starts_with(b"<svg"), "{path} is not an SVG");
        }
    }
}

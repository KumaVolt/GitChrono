//! Shown instead of the tabs when no usable git is found: what's wrong and how
//! to install or update git on this system, then "Check again".

use gitchrono_core::gitbin::{GitCheck, MIN_VERSION};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::shell::Shell;
use crate::theme::{self, c};
use crate::ui::button;

/// The install command for this Linux distribution, from /etc/os-release.
pub fn linux_install_command(os_release: &str) -> &'static str {
    let field = |key: &str| {
        os_release.lines().find_map(|l| l.strip_prefix(key).map(|v| v.trim_matches('"').to_lowercase())).unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID="), field("ID_LIKE="));
    let has = |name: &str| ids.split_whitespace().any(|w| w == name);
    if has("arch") || has("manjaro") {
        "sudo pacman -S git"
    } else if has("fedora") || has("rhel") || has("centos") {
        "sudo dnf install git"
    } else if has("opensuse") || has("suse") || ids.contains("opensuse") {
        "sudo zypper install git"
    } else if has("alpine") {
        "sudo apk add git"
    } else if has("void") {
        "sudo xbps-install git"
    } else if has("nixos") {
        "nix-env -iA nixpkgs.git"
    } else {
        "sudo apt install git"
    }
}

pub fn render(check: &GitCheck, cx: &mut Context<Shell>) -> impl IntoElement {
    let (title, detail): (&str, String) = match check {
        GitCheck::TooOld { path, version } => (
            "Git needs an update",
            format!("GitChrono needs Git {}.{} or newer, but found {version} at {}.", MIN_VERSION.0, MIN_VERSION.1, path.display()),
        ),
        _ => (
            "GitChrono needs Git",
            "GitChrono uses the Git on your computer for commits, pushes and everything else that changes a repository, so your settings, hooks and SSH keys work as usual. It couldn't find Git.".into(),
        ),
    };
    let label = |text: &'static str| div().text_xs().font_weight(FontWeight::BOLD).text_color(c(theme::MUTED)).child(text);
    let command = |text: &'static str| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().px_3().py_2().rounded_md().bg(c(theme::BG)).border_1().border_color(c(theme::BORDER)).font_family("monospace").text_sm().child(text))
            .child(button(SharedString::from(format!("copy-{text}"))).label("Copy").small().ghost().on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
            }))
    };

    let steps = if cfg!(target_os = "macos") {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(label("INSTALL ON MACOS"))
            .child(div().text_sm().text_color(c(theme::MUTED)).child("Apple's Command Line Tools include Git. Click below, then Install in the window that opens; it takes a few minutes."))
            .child(div().flex().child(button("git-install-clt").label("Install Command Line Tools").small().primary().on_click(|_, _, _| {
                let _ = std::process::Command::new("xcode-select").arg("--install").spawn();
            })))
            .child(div().text_sm().text_color(c(theme::MUTED)).child("Or, with Homebrew:"))
            .child(command("brew install git"))
    } else if cfg!(windows) {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(label("INSTALL ON WINDOWS"))
            .child(div().text_sm().text_color(c(theme::MUTED)).child(
                "GitChrono normally brings its own Git, but it's missing from this copy. Install Git for Windows, or download GitChrono again.",
            ))
            .child(div().flex().gap_2().child(button("git-download-win").label("Download Git for Windows").small().primary().on_click(|_, _, cx| {
                cx.open_url("https://git-scm.com/download/win")
            })))
            .child(div().text_sm().text_color(c(theme::MUTED)).child("Or with winget:"))
            .child(command("winget install --id Git.Git -e"))
    } else {
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(label("INSTALL ON LINUX"))
            .child(div().text_sm().text_color(c(theme::MUTED)).child("Install Git with your package manager:"))
            .child(command(linux_install_command(&os_release)))
    };

    div().id("git-help").size_full().overflow_y_scroll().flex().justify_center().items_start().px_8().py_10().bg(c(theme::BG)).child(
        div()
            .w_full()
            .max_w(px(640.))
            .flex()
            .flex_col()
            .gap_4()
            .child(div().flex().items_center().gap_3().child(img(crate::shell::logo()).size(px(44.)).flex_none()).child(div().text_2xl().font_weight(FontWeight::BOLD).child(title)))
            .child(div().text_sm().text_color(c(theme::MUTED)).child(detail))
            .child(div().p_4().rounded_lg().border_1().border_color(c(theme::BORDER)).bg(c(theme::PANEL)).child(steps))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(button("git-check-again").label("Check again").primary().on_click(cx.listener(|this, _, window, cx| this.recheck_git(window, cx))))
                    .child(div().text_xs().text_color(c(theme::FAINT)).child("Your open tabs come back once Git is found.")),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::linux_install_command;

    #[test]
    fn picks_the_package_manager() {
        assert_eq!(linux_install_command("ID=ubuntu\nID_LIKE=debian\n"), "sudo apt install git");
        assert_eq!(linux_install_command("ID=fedora\n"), "sudo dnf install git");
        assert_eq!(linux_install_command("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"), "sudo dnf install git");
        assert_eq!(linux_install_command("ID=endeavouros\nID_LIKE=arch\n"), "sudo pacman -S git");
        assert_eq!(linux_install_command("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"), "sudo zypper install git");
        assert_eq!(linux_install_command(""), "sudo apt install git");
    }
}

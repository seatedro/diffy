use std::path::PathBuf;

use quark_components::{PaletteItem, PaletteProvider};
use quark_ui::element::Binding;

use crate::contracts::commands::{CommandDescriptor, CommandsAction};
use crate::contracts::navigation::{NavigationAction, SelectedFile, SelectionOrigin};
use crate::contracts::shared::FileKey;
use crate::contracts::shell::Action;

#[derive(Debug, Default)]
pub struct PaletteSource {
    pub commands: Vec<PaletteCommand>,
    pub files: Vec<(FileKey, String)>,
    pub repositories: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub descriptor: CommandDescriptor,
    pub binding: Option<Binding>,
}

pub fn command_key(id: &str) -> u64 {
    quark::stable_hash(id)
}

pub fn frecency_key(id: &str) -> String {
    format!("cmd:{id}")
}

pub struct Commands;

impl PaletteProvider<PaletteSource> for Commands {
    fn title(&self) -> &str {
        "Commands"
    }

    fn collect(&self, source: &PaletteSource, out: &mut Vec<PaletteItem>) {
        out.extend(
            source
                .commands
                .iter()
                .filter(|c| c.descriptor.enabled)
                .map(|c| {
                    let d = &c.descriptor;
                    let mut item = PaletteItem::new(
                        command_key(d.id),
                        d.title.clone(),
                        Action::from(CommandsAction::RunCommand(d.id)),
                    );
                    if let Some(detail) = &d.detail {
                        item = item.subtitle(detail.clone());
                    }
                    if let Some(binding) = &c.binding {
                        item = item.binding(binding.clone());
                    }
                    item
                }),
        );
    }
}

pub struct Files;

impl PaletteProvider<PaletteSource> for Files {
    fn title(&self) -> &str {
        "Files"
    }

    fn collect(&self, source: &PaletteSource, out: &mut Vec<PaletteItem>) {
        out.extend(source.files.iter().map(|(key, path)| {
            PaletteItem::new(
                quark::stable_hash(path) ^ key.0.rotate_left(17),
                path.clone(),
                Action::from(NavigationAction::SelectFile(SelectedFile {
                    key: *key,
                    origin: SelectionOrigin::Command,
                })),
            )
        }));
    }
}

pub struct Repositories;

impl PaletteProvider<PaletteSource> for Repositories {
    fn title(&self) -> &str {
        "Repositories"
    }

    fn collect(&self, source: &PaletteSource, out: &mut Vec<PaletteItem>) {
        out.extend(source.repositories.iter().map(|path| {
            let display = path.display().to_string();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| display.clone());
            PaletteItem::new(
                quark::stable_hash(&display),
                name,
                Action::from(NavigationAction::OpenRepository(path.clone())),
            )
            .subtitle(display)
        }));
    }
}

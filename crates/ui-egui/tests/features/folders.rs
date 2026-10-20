use super::*;

#[test]
fn folders_are_a_normal_home_section_with_a_clear_add_action() {
    let f = Features::only(&[]);
    let mut h = open_models(&f);

    h.get_by_label("Folders");
    h.get_by_label("Choose where new model files should go. Existing files stay where they are.");
    h.get_by_role_and_label(Role::Button, "Add a folder").click();
    settle(&mut h);
    assert!(f.0.borrow().folder_calls.iter().any(|call| call == "add"));
}

#[test]
fn folder_actions_are_disabled_while_the_list_is_updating() {
    let f = Features::only(&[]);
    {
        let mut world = f.0.borrow_mut();
        world.folders = artcraft_launcher_ui_egui::features::FoldersSnapshot {
            loaded: true,
            busy: true,
            folders: vec![model_folder("extra", "Extra models", "C:/Models/Extra", false, craft_ai_model::model_folders::FolderOrigin::Chosen)],
            ..Default::default()
        };
    }
    let h = open_models(&f);

    h.get_by_label("Updating folders...");
    for label in ["Add a folder", "Make default", "Rescan", "Remove"] {
        assert!(h.get_by_role_and_label(Role::Button, label).accesskit_node().is_disabled(), "{label}");
    }
}

#[test]
fn folder_rows_rescan_and_remove_safely_and_keep_comfyui_read_only() {
    let f = Features::only(&[]);
    {
        let mut world = f.0.borrow_mut();
        world.folders = artcraft_launcher_ui_egui::features::FoldersSnapshot {
            loaded: true,
            folders: vec![
                model_folder("extra", "Extra models", "C:/Models/Extra", false, craft_ai_model::model_folders::FolderOrigin::Chosen),
                model_folder("comfy", "ComfyUI models", "C:/ComfyUI/models", false, craft_ai_model::model_folders::FolderOrigin::ComfyUi),
            ],
            ..Default::default()
        };
    }
    let mut h = open_models(&f);
    h.get_by_label("Extra models");
    h.get_by_label("Read-only");
    assert_eq!(h.query_all_by_role_and_label(Role::Button, "Make default").count(), 1);

    h.get_by_role_and_label(Role::Button, "Make default").click();
    settle(&mut h);
    h.query_all_by_role_and_label(Role::Button, "Rescan").next().unwrap().click();
    settle(&mut h);
    h.query_all_by_role_and_label(Role::Button, "Remove").next().unwrap().click();
    settle(&mut h);
    let calls = &f.0.borrow().folder_calls;
    assert!(calls.iter().any(|call| call == "default:extra"));
    assert!(calls.iter().any(|call| call == "rescan:extra"));
    assert!(calls.iter().any(|call| call == "remove:extra"));
}

#[test]
fn a_folder_problem_explains_the_recovery_step() {
    let f = Features::only(&[]);
    {
        let mut world = f.0.borrow_mut();
        world.folders.loaded = true;
        world.folders.notice = Some(artcraft_launcher_ui_egui::features::FolderNotice::Failed);
    }
    let h = open_models(&f);

    h.get_by_label("That folder could not be changed. Check that it is available, then try again.");
}

fn model_folder(
    id: &str,
    label: &str,
    path: &str,
    default: bool,
    origin: craft_ai_model::model_folders::FolderOrigin,
) -> craft_ai_model::model_folders::ModelFolder {
    craft_ai_model::model_folders::ModelFolder { id: id.into(), label: label.into(), path: path.into(), default, origin, kinds: Default::default() }
}

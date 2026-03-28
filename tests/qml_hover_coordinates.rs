use std::fs;
use std::path::Path;

#[test]
fn diff_pane_hover_overlay_stays_in_viewport_coordinates() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let qml = fs::read_to_string(repo_root.join("qml/components/DiffPane.qml")).unwrap();

    let flickable_index = qml.find("Flickable {").unwrap();
    let surface_index = qml.find("DiffSurface {").unwrap();
    let mouse_area_index = qml.rfind("MouseArea {").unwrap();

    assert!(surface_index > flickable_index);
    assert!(mouse_area_index > surface_index);
    assert!(qml.contains("surface.hoverY = mouse.y"));
    assert!(!qml.contains("surface.hoverY = mouse.y - diffViewport.contentY"));
    assert!(!qml.contains("surface.hoverY = mouseY - diffViewport.contentY"));
}

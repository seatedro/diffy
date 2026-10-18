use quark_app::testing::By;
use quark_ui::accessibility::AccessibilityRole as Role;
use quark_ui::text_input::TextField;

use crate::app::testing::{Harness, harness};
use crate::app::text_targets::SIDEBAR_FILTER;

fn filter(ui: &Harness) -> &TextField {
    ui.app().controllers().navigation.filter()
}

#[test]
fn filter_field_takes_typing_and_ime_and_keeps_focus() {
    let mut ui = harness();
    let field = By::role_name(Role::SearchInput, "Filter files");

    ui.click_node(field.clone());
    assert_eq!(ui.focus(), Some(SIDEBAR_FILTER));

    ui.type_text("src");
    assert_eq!(filter(&ui).text(), "src");
    assert_eq!(ui.find(field.clone()).value.as_deref(), Some("src"));
    ui.find(By::name("No matching files"));

    ui.ime_preedit("ü", Some((0, 2)));
    assert!(filter(&ui).preedit().is_some());
    assert_eq!(filter(&ui).text(), "src");
    assert!(ui.ime().allowed);

    ui.ime_commit("ü");
    assert_eq!(filter(&ui).text(), "srcü");
    assert!(filter(&ui).preedit().is_none());
    assert_eq!(ui.focus(), Some(SIDEBAR_FILTER));
    assert!(ui.find(field).focused);
    assert!(ui.painted_text().contains("srcü"));
}

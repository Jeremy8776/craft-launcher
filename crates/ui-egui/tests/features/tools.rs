use super::*;

// ---- Advanced: which model each tool uses ----

fn choice(connection: &str, model: &str, label: &str, local: bool) -> ToolChoice {
    ToolChoice {
        connection: ConnectionId::new(connection).unwrap(),
        model: ModelId::new(model).unwrap(),
        provider: if local { "comfyui" } else { "openai" }.into(),
        label: label.into(),
        local,
    }
}

fn tool(feature: &str, choices: Vec<ToolChoice>) -> ToolState {
    ToolState { feature: feature.into(), choices, bound: None, status: ToolStatus::NoChoice, can_test: true, costly: false }
}

fn ready_image_tool(f: &Features) {
    with_image_choices(f);
    let mut w = f.0.borrow_mut();
    let t = &mut w.tools[0];
    t.bound = Some((ConnectionId::new("openai").unwrap(), ModelId::new("gpt-image-1").unwrap()));
    t.status = ToolStatus::Ready { label: "GPT Image 1, OpenAI".into(), local: false };
}

fn open_advanced(f: &Features) -> Harness<'static, State> {
    f.0.borrow_mut().feature_titles = Some(vec!["Generate images".into()]);
    let mut h = open(f);
    h.query_all_by_role_and_label(Role::Button, "Configure").next().unwrap().click();
    settle(&mut h);
    h
}

fn pick(h: &mut Harness<'_, State>, label: &str) {
    pickers(h)[0].click();
    settle(h);
    h.get_by_label_contains(label).click();
    settle(h);
    h.get_by_label("Apply").click();
    settle(h);
}

fn asking() -> Sample {
    Sample::NeedsConsent {
        feature: "generate-image".into(),
        connection: "OpenAI".into(),
        destination: "https://api.openai.com".into(),
        items: vec!["text".into()],
        billing: Billing::Metered,
    }
}

#[test]
fn tool_cards_open_an_inline_configure_panel_and_engines_have_an_empty_state() {
    let f = Features::default();
    f.0.borrow_mut().feature_titles = Some(vec!["Generate images".into()]);
    with_image_choices(&f);
    let mut h = open(&f);
    assert_eq!(h.query_all_by_role_and_label(Role::Button, "Configure").count(), 1);
    h.get_by_label("Generate images");
    h.query_all_by_role_and_label(Role::Button, "Configure").next().unwrap().click();
    settle(&mut h);
    h.get_by_label("Model");
    h.get_by_label("Apply to");

    let empty = open(&Features::default());
    empty.get_by_role_and_label(Role::Button, "Engines");
}

#[test]
fn picking_a_model_binds_the_tool() {
    let f = Features::default();
    with_image_choices(&f);
    let mut h = open_advanced(&f);
    assert_eq!(picker_text(&h), "Automatic (best available)");
    pick(&mut h, "GPT Image 1, OpenAI");
    assert_eq!(
        f.0.borrow().chosen,
        [("generate-image".to_owned(), Some((ConnectionId::new("openai").unwrap(), ModelId::new("gpt-image-1").unwrap())))]
    );
    h.get_by_label("Ready. Sends to a hosted service.");
}

#[test]
fn a_local_model_is_marked_as_staying_on_this_computer() {
    let f = Features::default();
    with_image_choices(&f);
    {
        let mut w = f.0.borrow_mut();
        let t = &mut w.tools[0];
        t.bound = Some((ConnectionId::new("local").unwrap(), ModelId::new("flux").unwrap()));
        t.status = ToolStatus::Ready { label: "Flux, My ComfyUI".into(), local: true };
    }
    let mut h = open_advanced(&f);
    h.get_by_label("Ready. Runs on this computer.");
    let model_picker = h.get_by_value("Flux, My ComfyUI");
    model_picker.scroll_to_me();
    settle(&mut h);
    h.get_by_value("Flux, My ComfyUI").click();
    settle(&mut h);
    h.query_all_by_label_contains("Flux, My ComfyUI · Runs on this computer").next().unwrap();
}

#[test]
fn changing_runtime_keeps_the_workflow_model_and_only_changes_that_workflow() {
    let f = Features::default();
    with_image_choices(&f);
    {
        let mut world = f.0.borrow_mut();
        let t = &mut world.tools[0];
        t.choices.push(ToolChoice {
            connection: ConnectionId::new("native").unwrap(),
            model: ModelId::new("flux").unwrap(),
            provider: "candle".into(),
            label: "Flux, built in".into(),
            local: true,
        });
        t.bound = Some((ConnectionId::new("local").unwrap(), ModelId::new("flux").unwrap()));
        t.status = ToolStatus::Ready { label: "Flux, My ComfyUI".into(), local: true };
    }
    let mut h = open_advanced(&f);
    let runtime = h.get_by_value("ComfyUI");
    runtime.click();
    settle(&mut h);
    h.get_by_label("Inside this app").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().chosen, [("generate-image".to_owned(), Some((ConnectionId::new("native").unwrap(), ModelId::new("flux").unwrap())))]);
}

#[test]
fn a_runtime_that_cannot_run_the_selected_model_is_not_activated() {
    let f = Features::default();
    with_image_choices(&f);
    {
        let mut world = f.0.borrow_mut();
        let t = &mut world.tools[0];
        t.choices.push(ToolChoice {
            connection: ConnectionId::new("native").unwrap(),
            model: ModelId::new("other").unwrap(),
            provider: "candle".into(),
            label: "Other model, built in".into(),
            local: true,
        });
        t.bound = Some((ConnectionId::new("local").unwrap(), ModelId::new("flux").unwrap()));
        t.status = ToolStatus::Ready { label: "Flux, My ComfyUI".into(), local: true };
    }
    let mut h = open_advanced(&f);
    h.get_by_value("ComfyUI").click();
    settle(&mut h);
    let unavailable = h.get_by_label("Inside this app");
    assert!(unavailable.accesskit_node().is_disabled());
    h.get_by_label("This model is not available with every engine.");
    assert!(f.0.borrow().chosen.is_empty(), "the existing ComfyUI binding must remain active");
}

#[test]
fn a_refusal_from_the_shell_is_shown_and_nothing_changes() {
    let f = Features::default();
    with_image_choices(&f);
    f.0.borrow_mut().choose_refused = Some("That model cannot do this job.".into());
    let mut h = open_advanced(&f);
    pick(&mut h, "GPT Image 1, OpenAI");
    assert!(f.0.borrow().chosen.is_empty());
    h.get_by_label_contains("That model cannot do this job.");
}

#[test]
fn a_choice_that_stopped_working_says_why_and_automatic_takes_the_best_again() {
    let f = Features::default();
    with_image_choices(&f);
    {
        let mut w = f.0.borrow_mut();
        let t = &mut w.tools[0];
        t.bound = Some((ConnectionId::new("gone").unwrap(), ModelId::new("gpt-image-1").unwrap()));
        t.status = ToolStatus::Unavailable("Connection OpenAI is switched off.".into());
    }
    let mut h = open_advanced(&f);
    assert_eq!(picker_text(&h), "Chosen model unavailable");
    h.get_by_label("Connection OpenAI is switched off.");
    assert!(h.get_by_label("Test it").accesskit_node().is_disabled(), "no test of a model that cannot run");
    pickers(&h)[0].click();
    settle(&mut h);
    h.get_by_label("Automatic (best available)").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().chosen, [("generate-image".to_owned(), None)]);
}

#[test]
fn test_it_is_off_until_a_working_model_is_chosen() {
    let f = Features::default();
    with_image_choices(&f);
    let h = open_advanced(&f);
    assert!(h.get_by_label("Test it").accesskit_node().is_disabled());
}

#[test]
fn test_it_runs_the_sample_and_shows_that_it_worked() {
    let f = Features::default();
    ready_image_tool(&f);
    let mut h = open_advanced(&f);
    let test_button = h.get_by_label("Test it");
    test_button.scroll_to_me();
    settle(&mut h);
    h.get_by_label("Test it").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().tests, [("generate-image".to_owned(), false)]);
    h.get_by_label("Testing…");
    f.0.borrow_mut().sample = Sample::Passed { feature: "generate-image".into(), summary: "Returned 1 image result(s)".into(), millis: 1_250 };
    settle(&mut h);
    h.get_by_label("Works. Returned 1 image result(s), in 1.3 s.");
    let done = h.query_all_by_role_and_label(Role::Button, "Done").last().unwrap();
    done.scroll_to_me();
    settle(&mut h);
    h.query_all_by_role_and_label(Role::Button, "Done").last().unwrap().click();
    settle(&mut h);
    assert_eq!(f.0.borrow().dismissals, 1);
    h.get_by_label("Test it");
}

#[test]
fn a_failed_test_says_why_and_can_be_tried_again() {
    let f = Features::default();
    ready_image_tool(&f);
    f.0.borrow_mut().sample = Sample::Failed { feature: "generate-image".into(), message: "the service said no".into() };
    let mut h = open_advanced(&f);
    h.get_by_label("The test failed: the service said no");
    let retry = h.get_by_label("Try again");
    retry.scroll_to_me();
    settle(&mut h);
    h.get_by_label("Try again").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().tests, [("generate-image".to_owned(), false)]);
}

#[test]
fn a_hosted_test_asks_first_and_names_where_it_goes_and_what_it_costs() {
    let f = Features::default();
    ready_image_tool(&f);
    f.0.borrow_mut().sample = asking();
    let mut h = open_advanced(&f);
    h.get_by_label("This sends a small sample (text) to https://api.openai.com through OpenAI. You are billed for it by the service.");
    assert!(f.0.borrow().tests.is_empty(), "nothing was sent by looking at the question");
    let cancel = h.get_by_label("Cancel");
    cancel.scroll_to_me();
    settle(&mut h);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().dismissals, 1);
    assert!(f.0.borrow().tests.is_empty());
    // Asked again, agreeing sends it.
    f.0.borrow_mut().sample = asking();
    settle(&mut h);
    h.get_by_label("Send the test").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().tests, [("generate-image".to_owned(), true)]);
}

#[test]
fn a_running_test_can_be_cancelled() {
    let f = Features::default();
    ready_image_tool(&f);
    f.0.borrow_mut().sample = Sample::Running { feature: "generate-image".into() };
    let mut h = open_advanced(&f);
    h.get_by_label("Testing…");
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().test_cancels, 1);
}

#[test]
fn a_sample_for_one_tool_does_not_show_on_another() {
    let f = Features::default();
    ready_image_tool(&f);
    {
        let mut w = f.0.borrow_mut();
        let mut video = tool("generate-video", vec![choice("openai", "sora", "Sora, OpenAI", false)]);
        video.bound = Some((ConnectionId::new("openai").unwrap(), ModelId::new("sora").unwrap()));
        video.status = ToolStatus::Ready { label: "Sora".into(), local: false };
        video.costly = true;
        w.tools.push(video);
        w.feature_titles = Some(vec!["Generate video".into()]);
        w.sample = Sample::Passed { feature: "generate-image".into(), summary: "Returned 1 image result(s)".into(), millis: 900 };
    }
    let h = open(&f);
    assert_eq!(h.query_all_by_label_contains("Works.").count(), 0, "another tool's sample is hidden");
    h.get_by_label("Generate video");
}

#[test]
fn setup_window_uses_the_requested_tool_first_navigation() {
    let h = open(&Features::default());
    for label in ["Tools", "Usage", "Connections", "Models", "Engines"] {
        h.get_by_role_and_label(Role::Button, label);
    }
    h.get_by_role_and_label(Role::Button, "Tools");
}

#[test]
fn a_connected_server_says_so_in_connections() {
    let f = Features::default();
    let c = Connections::default();
    let mut ollama = saved_preset("ollama");
    ollama.models = 3;
    c.0.borrow_mut().rows = vec![ollama];

    let h = open_connections(&f, &c);
    h.get_by_label("Ollama");
    h.get_by_label("This computer");
    h.get_by_label("Models: 3");
}

#[test]
fn a_connected_server_with_no_models_yet_says_that_instead_of_a_count() {
    let f = Features::default();
    let c = Connections::default();
    c.0.borrow_mut().rows = vec![saved_preset("ollama")];
    let h = open_connections(&f, &c);
    h.get_by_label("Models: 0");
}

#[test]
fn setup_ai_has_the_five_requested_navigation_sections() {
    let h = open(&Features::default());
    for label in ["Tools", "Usage", "Connections", "Models", "Engines"] {
        h.get_by_role_and_label(Role::Button, label);
    }
}

#[test]
fn configure_opens_model_scope_and_engine_choices_in_the_tool_list() {
    let f = Features::default();
    f.0.borrow_mut().feature_titles = Some(vec!["Generate images".into()]);
    with_image_choices(&f);
    f.0.borrow_mut().tools[0].choices.push(choice("native", "flux", "Flux, built in", true));
    let mut h = open(&f);
    h.query_all_by_role_and_label(Role::Button, "Configure").next().unwrap().click();
    settle(&mut h);
    h.get_by_value("Automatic (best available)").click();
    settle(&mut h);
    h.get_by_label("Flux, built in · Runs on this computer").click();
    settle(&mut h);
    h.get_by_label("Apply to");
    h.get_by_value("This tool").click();
    settle(&mut h);
    for label in ["This tool", "All compatible tools", "Choose specific tools"] {
        h.get_by_label(label);
    }
    h.get_by_label("Engine");
    h.get_by_label("Apply");
}

#[test]
fn sam3_files_in_place_are_not_reported_as_a_tested_tool() {
    let f = Features::default();
    f.0.borrow_mut().inventory = with_sam3();
    let h = open_models(&f);
    h.get_by_label("Files in place");
    // SAM 2.1 runs in PhotoCraft; SAM 2 and SAM 1 are setup only, and say so once per tool.
    assert_eq!(h.query_all_by_label("Setup only").count(), 2, "only SAM 2 and SAM 1 have no app route");
    assert_eq!(h.query_all_by_label("Models marked Setup only can be installed, but no Craft app can use them yet.").count(), 1);
    let tools = open(&f);
    assert!(tools.query_all_by_label_contains("Suggested for PhotoCraft:").next().is_some());
}
